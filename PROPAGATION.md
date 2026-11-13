# Propagation plan

How btop-web becomes known and used: a demo first, then posts to the audiences that already run btop, then the listings people search, then trial reduced to one command.

## Order

1. Demo - a read-only public container on a small VPS, fed by the fixture tree rather than a live host, and a 10 s GIF of the live stream at the top of the README. Posts that link a demo convert; posts that link a repository do not
2. Posts, one per week - Hacker News "Show HN: btop in the browser - a Rust port of btop's telemetry" with the demo link in the first line; r/selfhosted and r/homelab with the phone-dashboard use case; then r/rust and r/commandline for the technical angle; Lobsters after
3. Listings - an awesome-selfhosted PR is the largest single inflow for self-hosted tools; then awesome-rust, awesome-docker, AlternativeTo under "btop alternatives", and a submission to selfh.st's weekly
4. Upstream mention - a "related projects" line in aristocratos/btop's README or a maintainer answer in its discussions. Ask after the demo exists; it reaches exactly the target users and outranks self-posts
5. One-command trial - publish the image on Docker Hub as well as ghcr.io, and put the run line at the top of the README:

   `docker run --rm --pid=host -p 40290:40290 ghcr.io/stellarshenson/btop-web:1.0`

6. Repository surface - GitHub description and topics set `btop`, `monitoring`, `dashboard`, `rust`, `self-hosted`; issue templates; two or three "good first issue" labels; a GitHub Release per tag so the Releases page exists
7. Measure - GitHub traffic referrers and crates.io downloads name the channel that paid; feed that channel again

## Constraints

- The demo never exposes the live host: fixtures or a throwaway container, `--readonly`
- The README names the project unofficial, because the name borrows btop's

## Prepared here, sent by hand

- The demo compose file, the README trial block, the topic list and post drafts can be produced as a workflow
- The posts and the upstream request are sent by the owner
