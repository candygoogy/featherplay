#!/usr/bin/env python3
"""A small HTTP proxy on 127.0.0.1 that writes down every request it gets.

Usage: proxy.py <port> <requests.jsonl> forward|deny

- forward: CONNECT tunnels to the host asked for, and plain requests are passed on;
- deny: every request is answered 403 and nothing leaves the machine.

Each request is one JSON line: {"method": "CONNECT", "target": "github.com:443", "answer": ...}.
Runs until killed. Standard library only.
"""

import asyncio
import json
import sys
import time

port, log_path, mode = int(sys.argv[1]), sys.argv[2], sys.argv[3]
log = open(log_path, "a", buffering=1, encoding="utf-8")


def record(**fields):
    fields["at"] = round(time.time(), 3)
    log.write(json.dumps(fields) + "\n")


async def pipe(reader, writer):
    try:
        while data := await reader.read(65536):
            writer.write(data)
            await writer.drain()
    except (ConnectionError, asyncio.CancelledError):
        pass
    finally:
        writer.close()


async def handle(reader, writer):
    try:
        head = await reader.readuntil(b"\r\n\r\n")
    except (asyncio.IncompleteReadError, asyncio.LimitOverrunError, ConnectionError):
        writer.close()
        return
    lines = head.decode("latin-1").split("\r\n")
    method, target, _ = (lines[0].split(" ") + ["", ""])[:3]
    if mode != "forward":
        record(method=method, target=target, answer=403)
        writer.write(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        await writer.drain()
        writer.close()
        return
    try:
        if method == "CONNECT":
            host, _, p = target.rpartition(":")
            up_reader, up_writer = await asyncio.open_connection(host, int(p))
            writer.write(b"HTTP/1.1 200 Connection established\r\n\r\n")
            await writer.drain()
        else:
            # http://host[:port]/path: passed on as it came, to that host.
            rest = target.split("://", 1)[-1]
            hostport = rest.split("/", 1)[0]
            host, _, p = hostport.partition(":")
            up_reader, up_writer = await asyncio.open_connection(host, int(p or 80))
            up_writer.write(head)
            await up_writer.drain()
    except (OSError, ValueError) as e:
        record(method=method, target=target, answer=502, error=str(e))
        writer.write(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        await writer.drain()
        writer.close()
        return
    record(method=method, target=target, answer=200)
    await asyncio.gather(pipe(reader, up_writer), pipe(up_reader, writer))


async def main():
    server = await asyncio.start_server(handle, "127.0.0.1", port)
    record(method="-", target=f"listening on {port} ({mode})", answer=0)
    async with server:
        await server.serve_forever()


asyncio.run(main())
