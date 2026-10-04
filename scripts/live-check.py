#!/usr/bin/env python3
"""Run every protonctl tool once over MCP, as the Claude app does, against this
Mac's real calendar link, Bridge and Drive. Prints one line per call with its
outcome, time and counts, never message, event or file content or names.

Run it from a normal terminal, since codesign misreports inside Claude Code's
sandbox, against the installed binary, which the Claude app also runs, so one
Keychain approval covers both:

    python3 scripts/live-check.py [path/to/protonctl]

A new build's first calendar read and mail login each ask for Keychain access,
and protonctl waits up to 150 s for the answer (R8).
"""

import json
import os
import queue
import subprocess
import sys
import threading
import time

BIN = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/.cargo/bin/protonctl")
WAIT = 200  # seconds per call: past protonctl's own 150 s limit
# Results that carry third-party text must say so in `provenance` (R6).
CONTENT_TOOLS = {
    "list_events", "search_events", "get_event", "search_files", "list_folder",
    "get_file_metadata", "read_file_content", "download_file", "search_threads",
    "count_messages", "get_message", "get_thread", "get_attachment",
}

server = subprocess.Popen(
    [BIN, "serve"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
)
lines: queue.Queue = queue.Queue()
threading.Thread(target=lambda: [lines.put(line) for line in server.stdout], daemon=True).start()
last_id = 0
failures: list[str] = []
called: set[str] = set()


def send(message: dict) -> None:
    server.stdin.write(json.dumps(message) + "\n")
    server.stdin.flush()


def rpc(method: str, params: dict | None = None) -> dict:
    global last_id
    last_id += 1
    send({"jsonrpc": "2.0", "id": last_id, "method": method, "params": params or {}})
    deadline = time.monotonic() + WAIT
    while True:
        reply = json.loads(lines.get(timeout=max(0.1, deadline - time.monotonic())))
        if reply.get("id") == last_id:
            return reply


def counts(data: dict) -> str:
    """List lengths and a few flags, never values."""
    parts = [f"{k} {len(v)}" for k, v in data.items() if isinstance(v, list)]
    for flag in ("complete", "truncated", "bodyTruncated", "hiddenCharactersRemoved", "skippedNames"):
        if data.get(flag) is not None:
            parts.append(f"{flag} {data[flag]}")
    return ", ".join(parts)


def calendar_note(data: dict) -> str:
    """R12: every calendar read says when it was fetched, plus the freshness notice."""
    sources = data.get("calendars") or []
    fetched = sum("fetchedAt" in s for s in sources)
    if fetched < len(sources):
        failures.append("a calendar could not be read")
    return counts(data) + f", fetchedAt {fetched}/{len(sources)}, freshness {'yes' if data.get('freshness') else 'NO'}"


def call(name: str, args: dict, describe=counts, label: str = "") -> dict | None:
    called.add(name)
    shown = f"{name}{f' ({label})' if label else ''}"
    started = time.monotonic()
    reply = rpc("tools/call", {"name": name, "arguments": args})
    took = time.monotonic() - started
    result = reply.get("result", {})
    text = (result.get("content") or [{}])[0].get("text", "")
    if "error" in reply or result.get("isError"):
        failures.append(shown)
        print(f"FAIL  {shown:<34} {took:5.1f}s  {(reply.get('error', {}).get('message') or text)[:160]}")
        return None
    data = json.loads(text)
    note = describe(data)
    if name in CONTENT_TOOLS and "provenance" not in data:
        failures.append(f"{shown}: no provenance")
        note += "  [no provenance]"
    print(f"ok    {shown:<34} {took:5.1f}s  {note}")
    return data


init = rpc("initialize", {
    "protocolVersion": "2025-11-25", "capabilities": {},
    "clientInfo": {"name": "live-check", "version": "0"},
})["result"]
send({"jsonrpc": "2.0", "method": "notifications/initialized"})
tools = {t["name"] for t in rpc("tools/list")["result"]["tools"]}
print(f"{init['serverInfo']['name']} {init['serverInfo']['version']}: {len(tools)} tools\n")

def digested(d: dict) -> bool:
    """Downloads and attachments carry their digests."""
    ok = len(d.get("sha256") or "") == 64 and len(d.get("sha1") or "") == 40
    if not ok:
        failures.append("a saved file without sha256 and sha1")
    return ok


status = call("get_status", {}, lambda d: (
    f"calendars {len(d['calendars'])}, mail {'set up' if isinstance(d['mail'], dict) else 'not set up'}, "
    f"drive folder {'yes' if isinstance(d['drive'].get('folder'), str) else 'no'}, "
    f"secrets held {len(d['secretsHeld'])}"
))
export_folder = ((status or {}).get("export") or {}).get("folder") if isinstance((status or {}).get("export"), dict) else None
call("list_calendars", {})

# Calendar: the coming week, then a search over the default window for an eventId.
week = call("list_events", {}, calendar_note)
found = call("search_events", {"query": "e"}, calendar_note)
events = (week or {}).get("events") or (found or {}).get("events") or []
if events:
    call("get_event", {"eventId": events[0]["eventId"]}, lambda d: f"fields {len(d.get('event') or {})}")
else:
    print("skip  get_event                           no event in the coming week or the search window")

# Drive: search for a small text file, list the top folder, then read and download one file.
texts = call("search_files", {"query": "*.txt", "kind": "file"})
top = call("list_folder", {})
files = [f for f in (texts or {}).get("files", []) if 0 < (f.get("size") or 0) <= 256 * 1024]
files += [f for f in (top or {}).get("items", []) if f.get("kind") == "file"]
if files:
    path = files[0]["path"]
    call("get_file_metadata", {"path": path}, lambda d: f"kind {d['file']['kind']}, size given {d['file']['size'] is not None}")
    call("read_file_content", {"path": path}, lambda d: (
        f"chars {len(d['content'])}, truncated {d['truncated']}" if d.get("content") is not None
        else f"no content: {d.get('reason')}"
    ))
    call("download_file", {"path": path}, lambda d: (
        f"saved {'yes' if os.path.isfile(d['savedTo']) else 'NO'}, "
        f"size matches {os.path.getsize(d['savedTo']) == d['file']['size'] if os.path.isfile(d['savedTo']) else 'n/a'}, "
        f"sha256 {'yes' if digested(d) else 'NO'}"
    ))
    if export_folder:
        exported = call("download_file", {"path": path, "export": True}, lambda d: (
            f"in the export folder {'yes' if d['savedTo'].startswith(export_folder) else 'NO'}, sha256 {'yes' if digested(d) else 'NO'}"
        ), label="export")
        if exported and exported["savedTo"].startswith(export_folder):
            os.remove(exported["savedTo"])  # the check removes what it wrote
else:
    print("skip  get_file_metadata, read, download    no file found")
# A file that cannot come back as text: over 1 MiB, else a PDF.
large = [f for f in (top or {}).get("items", []) if f.get("kind") == "file" and (f.get("size") or 0) > 1024 * 1024]
if not large:
    large = (call("search_files", {"query": "*.pdf", "kind": "file", "pageSize": 1}, label="*.pdf") or {}).get("files") or []
if large:
    call("read_file_content", {"path": large[0]["path"]}, lambda d: (
        f"no content: {d.get('reason')}" if d.get("content") is None else f"chars {len(d['content'])}"
    ), label="not text")

# Mail: a recent search, one message and its thread, labels, then one attachment.
def row_shape(d: dict) -> str:
    """The row contract: 16-character messageIds, no encryption field."""
    rows = d.get("messages") or []
    long_ids = sum(len(r.get("messageId") or "") != 16 for r in rows)
    with_encryption = sum("encryption" in r for r in rows)
    if long_ids or with_encryption:
        failures.append(f"search rows: {long_ids} messageIds not 16 characters, {with_encryption} with encryption")
    return counts(d) + f", estimatedTotal {d.get('estimatedTotal')}, bad ids {long_ids}, encryption {with_encryption}"


recent = call("search_threads", {"query": "newer_than:30d", "pageSize": 5}, row_shape)
rows = (recent or {}).get("messages") or []
call("search_threads", {"query": "newer_than:30d", "pageSize": 10, "snippets": True}, lambda d: (
    row_shape(d) + f", snippets {sum(bool(r.get('snippet')) for r in d.get('messages') or [])}"
), label="snippets")
if rows:
    call("get_message", {"messageId": rows[0]["messageId"]}, lambda d: (
        f"body chars {len(d['message'].get('body') or '')}, attachments {len(d['message'].get('attachments') or [])}"
    ))
    if rows[0].get("threadId"):
        call("get_thread", {"threadId": rows[0]["threadId"]}, lambda d: f"total {d['total']}, shown {d['shown']}")
call("list_labels", {})
call("count_messages", {"query": "newer_than:30d", "by": "fromDomain"}, lambda d: (
    f"messages {d['messages']}, groups {d.get('groupsTotal')}, shown {len(d.get('groups') or [])}"
))
with_files = call("search_threads", {"query": "has:attachment newer_than:365d", "pageSize": 10}, label="has:attachment")
for row in (with_files or {}).get("messages") or []:
    message = call("get_message", {"messageId": row["messageId"]}, lambda d: f"attachments {len(d['message'].get('attachments') or [])}", label="for an attachment")
    attachments = (message or {}).get("message", {}).get("attachments") or []
    if attachments:
        call("get_attachment", {"messageId": row["messageId"], "index": attachments[0]["index"]}, lambda d: (
            f"size {d['size']}, type {(d.get('mimeType') or '?').split('/')[0]}, "
            f"saved {'yes' if os.path.isfile(d['path']) else 'NO'}, inline text {'yes' if d.get('text') else 'no'}, "
            f"sha256 {'yes' if digested(d) else 'NO'}"
        ))
        break

if export_folder:
    made = call("export_drive_manifest", {}, lambda d: f"rows {d['rows']}, complete {d['complete']}, sha256 {'yes' if d.get('sha256') else 'NO'}")
    if made and made["manifest"].startswith(export_folder):
        os.remove(made["manifest"])  # the check removes what it wrote
else:
    print("skip  export_drive_manifest                 no [export] folder configured")
untried = sorted(tools - called)
print(f"\ncalled {len(called)} of {len(tools)} tools" + (f"; not called: {', '.join(untried)}" if untried else ""))

# Stop the way a host does, then check the server cleaned up (R10).
server.stdin.close()
try:
    code = server.wait(timeout=15)
except subprocess.TimeoutExpired:
    server.kill()
    code = None
    failures.append("exit: did not stop within 15 s")
leftover = os.path.expanduser(f"~/Library/Caches/protonctl/downloads/{server.pid}")
if os.path.exists(leftover):
    failures.append("exit: download folder left behind")
stderr = server.stderr.read().splitlines()
print(f"server exited {code}; download folder removed {not os.path.exists(leftover)}; stderr lines {len(stderr)}")
print("all calls passed" if not failures else f"{len(failures)} problem(s): {'; '.join(failures)}")
sys.exit(1 if failures else 0)
