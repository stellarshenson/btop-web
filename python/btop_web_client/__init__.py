"""Client for the /events.json stream of btop-web, on the standard library alone."""

import hashlib
import http.client
import json
import ssl
from urllib.parse import urlsplit

__all__ = ["ClientError", "frames"]

STATUS_HINTS = {
    401: "pass the access key file of the agent",
    404: "client mode has no stream, connect to the agent itself",
}


class ClientError(Exception):
    """The arguments, the server's answer or the server's certificate were refused."""


def frames(url, key_file=None, pin=None, timeout=45.0):
    """Yield one parsed frame per line of the /events.json stream under ``url``.

    ``url`` is the base URL of the server. A standalone server takes ``http://``. An agent takes
    ``https://``, the path of its access key file and the ``sha256:`` fingerprint of its
    certificate; the fingerprint is checked before the key is sent. ``timeout`` bounds the
    connection and each read, in seconds.
    """
    parts = urlsplit(url)
    if parts.scheme not in ("http", "https") or not parts.hostname:
        raise ClientError(f"{url} is not an http:// or https:// URL")
    if parts.scheme == "http" and (key_file or pin):
        raise ClientError(f"a key file or a pin needs an https:// URL, not {url}")

    headers = {}
    if key_file:
        with open(key_file, encoding="utf-8") as file:
            key = file.read().strip()
        # A second line would put the key in http.client's invalid-header error.
        if len(key.splitlines()) != 1:
            raise ClientError(f"the access key file {key_file} must hold one line")
        headers["Authorization"] = f"Bearer {key}"

    if parts.scheme == "https":
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        context.minimum_version = ssl.TLSVersion.TLSv1_3
        # An agent's certificate is self-signed, so the fingerprint is the whole trust decision.
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE
        connection = http.client.HTTPSConnection(
            parts.hostname, parts.port, timeout=timeout, context=context
        )
    else:
        connection = http.client.HTTPConnection(parts.hostname, parts.port, timeout=timeout)

    try:
        connection.connect()
        if parts.scheme == "https":
            der = connection.sock.getpeercert(binary_form=True)
            presented = f"sha256:{hashlib.sha256(der).hexdigest()}"
            if pin is None:
                raise ClientError(
                    f"no pin given: {url} presented {presented}; "
                    "compare it with the agent's log line 'agent TLS certificate', then pin it"
                )
            if presented != pin:
                raise ClientError(
                    f"refused: {url} presented {presented}, the pin is {pin}; "
                    "ask the operator whether the agent's certificate changed"
                )
        connection.request("GET", f"{parts.path.rstrip('/')}/events.json", headers=headers)
        response = connection.getresponse()
        if response.status != 200:
            raise ClientError(
                f"{url} answered {response.status}: "
                f"{STATUS_HINTS.get(response.status, response.reason)}"
            )
        for line in response:
            yield json.loads(line)
    finally:
        connection.close()
