# Security Policy

## Supported versions

btop-web is pre-1.0. Only the latest release and the current `main` branch receive fixes.

## Reporting a vulnerability

Report vulnerabilities privately through GitHub Security Advisories: open the repository's **Security** tab and choose **Report a vulnerability**. Do not open a public issue.

Include the affected version, the deployment (loopback, reverse proxy, container), and the steps to reproduce.

The maintainer will review the report and coordinate a fix and disclosure with you. Please allow time for a release before publishing details.

## Scope

The dashboard exposes host telemetry: process names, command lines, user names, network endpoints and resource statistics. It has no authentication and binds to `127.0.0.1:40290` by default. Reports about data exposure caused by binding to a public interface without access control are deployment issues, and the documented remedy is a reverse proxy with TLS and authentication. Reports about memory safety, request handling, template escaping or path traversal in the embedded assets are in scope.
