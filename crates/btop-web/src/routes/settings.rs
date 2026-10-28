//! ACC-CONF-61-CONFIG-PAGES: the config menu's server rows and their write
//! path. With the read-only guard on, `readonly_guard` refuses the POST before
//! it reaches [`update`] and the rows render as text. With the guard off, a row
//! is an htmx form whose POST answers with the row re-rendered as HTML.

use std::collections::HashMap;

use askama::Template;
use axum::Form;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};

use crate::AppState;
use crate::config::{self, Config, ServerOption};

/// One server option as the menu shows it.
pub struct CfgRow {
    pub option: &'static ServerOption,
    pub value: String,
    /// The answer to this row's last POST; empty on a page render.
    pub msg: String,
}

impl CfgRow {
    pub fn new(option: &'static ServerOption, config: &Config, msg: String) -> Self {
        Self {
            option,
            value: config.get(option.key).unwrap_or_default(),
            msg,
        }
    }
}

#[derive(Template)]
#[template(path = "partials/cfg_key.html")]
struct CfgKey<'a> {
    row: &'a CfgRow,
    readonly: bool,
}

/// `POST config`: validate one `key=value` pair with the file's parser, write
/// the key's line to the loaded file, then hand the new configuration to the
/// sampler. Nothing changes unless every step succeeds.
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "cross-origin request refused").into_response();
    }
    let key = form.get("key").map(String::as_str).unwrap_or_default();
    let Some(option) = config::server_option(key) else {
        return (
            StatusCode::BAD_REQUEST,
            format!("not a server option: {key}"),
        )
            .into_response();
    };
    let file = state.config_file.lock().await;
    let current = state.config.borrow().clone();
    let refuse = |status, msg: String| row(status, CfgRow::new(option, &current, msg));
    let Some(value) = form.get("value") else {
        return refuse(StatusCode::UNPROCESSABLE_ENTITY, "refused: no value".into());
    };
    let mut candidate = current.clone();
    let line = match candidate.set(key, value) {
        Ok(line) => line,
        Err(error) => {
            return refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("refused: {error}"),
            );
        }
    };
    let msg = match file.clone() {
        Some(path) => {
            let saved = tokio::task::spawn_blocking(move || {
                config::save(&path, option.key, &line).map_err(|error| (path, error))
            })
            .await
            .expect("the save task does not panic");
            match saved {
                Ok(target) => format!("saved to {}", target.display()),
                Err((path, error)) => {
                    return refuse(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!(
                            "refused: could not write {}: {error}; nothing changed",
                            path.display()
                        ),
                    );
                }
            }
        }
        None => "applied until restart; no config file loaded".to_string(),
    };
    state.config.send_replace(candidate.clone());
    row(StatusCode::OK, CfgRow::new(option, &candidate, msg))
}

fn row(status: StatusCode, row: CfgRow) -> Response {
    match (CfgKey {
        row: &row,
        readonly: false,
    })
    .render()
    {
        Ok(html) => (status, Html(html)).into_response(),
        Err(error) => {
            tracing::error!(%error, "failed to render a config row");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// The request's `Origin` names the host it was sent to. jupyter-server-proxy
/// keeps `Host`, so both read the proxy's address behind the lab.
fn same_origin(headers: &HeaderMap) -> bool {
    let text = |name| headers.get(name).and_then(|value| value.to_str().ok());
    let (Some(origin), Some(host)) = (text(header::ORIGIN), text(header::HOST)) else {
        return false;
    };
    origin
        .parse::<Uri>()
        .ok()
        .and_then(|uri| {
            uri.authority()
                .map(|authority| authority.as_str().eq_ignore_ascii_case(host))
        })
        .unwrap_or(false)
}
