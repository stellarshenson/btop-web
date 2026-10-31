"""The btop-web-client command: print /events.json frames as JSON lines."""

import argparse
import http.client
import json
import os
import sys

from . import ClientError, frames

DESCRIPTION = """\
Read the /events.json stream of a btop-web server and print its frames to stdout, one JSON
document per line. Prints one frame and exits, or every frame with --follow. Reads the
telemetry only and changes nothing on the host.
"""

EPILOG = """\
output:
  One frame is about 130 KB on a host with 300 processes and grows with the process table, so
  keep the fields you need. It holds process command lines and user names, which can hold
  secrets. The README of btop-web gives the schema: units sit in the field suffix, and a value
  the server could not read is null, never zero.

connecting:
  Standalone server: http://HOST:PORT, no key and no pin.
  Agent (btop-web --agent): https://HOST:PORT, TLS 1.3, with --key-file and --pin. Unless the
  agent was given its own, its key is in $XDG_CONFIG_HOME/btop-web/access-key on its host. Its
  fingerprint is in its log line 'agent TLS certificate'. A missing or wrong pin is refused
  before the key is sent, and the refusal names the presented fingerprint. Compare that value
  with the agent's log line before you pin it.
  Client mode (btop-web --client) has no stream: connect to its agents.

exit codes:
  0    one frame printed, or the reader of --follow closed the pipe
  1    connection, TLS, pin, key or stream failure, named on stderr; --follow also ends here
       when the server closes the stream, and running the command again reconnects
  2    bad arguments
  130  interrupted with Ctrl-C

examples:
  btop-web-client http://127.0.0.1:40290
  btop-web-client http://127.0.0.1:40290 --follow | head -n 5
  btop-web-client https://host:40290 --key-file /keys/access-key --pin sha256:<64 hex digits>
"""


def main(argv=None):
    parser = argparse.ArgumentParser(
        prog="btop-web-client",
        description=DESCRIPTION,
        epilog=EPILOG,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("url", help="base URL of the server, without /events.json")
    parser.add_argument(
        "-f",
        "--follow",
        action="store_true",
        help="print every frame until stopped; runs until the pipe, the stream or Ctrl-C ends it",
    )
    parser.add_argument(
        "--key-file",
        metavar="PATH",
        help="file holding the agent's access key, sent as Authorization: Bearer; never printed",
    )
    parser.add_argument(
        "--pin",
        metavar="sha256:HEX",
        help="fingerprint the agent's certificate must have; required for https://",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=45.0,
        metavar="SECONDS",
        help="limit on the connection and on each read, default %(default)s; "
        "set it above the server's update_ms when that is longer",
    )
    args = parser.parse_args(argv)

    try:
        for frame in frames(args.url, args.key_file, args.pin, args.timeout):
            print(json.dumps(frame), flush=True)
            if not args.follow:
                return 0
        print("btop-web-client: the stream closed; run the command again", file=sys.stderr)
        return 1
    except BrokenPipeError:
        # The reader left, as `| head -n 5` does. The interpreter's exit flush must not fail too.
        os.dup2(os.open(os.devnull, os.O_WRONLY), sys.stdout.fileno())
        return 0
    except KeyboardInterrupt:
        return 130
    except http.client.BadStatusLine:
        # Its message is the raw bytes, which on an agent's listener are a TLS alert.
        print(
            f"btop-web-client: {args.url} did not answer HTTP; an agent takes https://",
            file=sys.stderr,
        )
        return 1
    except (ClientError, OSError, http.client.HTTPException, ValueError) as error:
        print(f"btop-web-client: {error}", file=sys.stderr)
        return 1
