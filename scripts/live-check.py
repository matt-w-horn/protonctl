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

import base64
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
    "get_file_metadata", "read_file_content", "download_file", "list_drive_tree", "search_threads",
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
blocks: list[dict] = []  # the content blocks of the last call, after its JSON


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
    blocks[:] = (result.get("content") or [])[1:]
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

def page(d: dict) -> str:
    """A page of text: its length, the whole length, and where the next one starts."""
    if d.get("content") is None:
        return f"no content: {d.get('reason')}"
    return (f"chars {len(d['content'])}, totalChars {d.get('totalChars')}, nextOffset {d.get('nextOffset')}, "
            f"from {d.get('textFrom')}")


def attached(kind: str, size: int | None = None) -> str:
    """The blocks after the JSON: images, or a base64 blob of `size` bytes."""
    block = blocks[0] if blocks else {}
    if kind == "image":
        ok = any(b.get("type") == "image" and b.get("data") for b in blocks)
    else:
        blob = (block.get("resource") or {}).get("blob") or ""
        ok = block.get("type") == "resource" and len(base64.b64decode(blob)) == size
    if not ok:
        failures.append(f"no {kind} block after the JSON")
    return f"{kind} block {'yes' if ok else 'NO'}"


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
    first = call("read_file_content", {"path": path}, page)
    if first and first.get("nextOffset"):
        call("read_file_content", {"path": path, "offset": first["nextOffset"]}, page, label="next page")
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
    small = [f for f in files if (f.get("size") or 0) <= 5 * 1024 * 1024]
    if small:
        call("download_file", {"path": small[0]["path"], "inline": True}, lambda d: (
            f"{attached('blob', d['inline']['bytes'])}, {'SAVED (unexpected)' if d.get('savedTo') else 'nothing saved'}, "
            f"sha256 {'yes' if digested(d) else 'NO'}"
        ), label="inline")
else:
    print("skip  get_file_metadata, read, download    no file found")
# A PDF's text, and an image as image content.
# Over 1 KB: a real PDF is larger than its own trailer.
pdfs = (call("search_files", {"query": "*.pdf", "kind": "file", "pageSize": 25}, label="*.pdf") or {}).get("files") or []
pdfs = [f for f in pdfs if (f.get("size") or 0) > 1024]
def pdf_pages(d: dict) -> str:
    """Page images: how many, each after a "Page N:" label, and which pages."""
    images = sum(b.get("type") == "image" for b in blocks)
    labels = sum(b.get("type") == "text" and b.get("text", "").startswith("Page ") for b in blocks)
    if not images or labels != images:
        failures.append(f"page images: {images} images, {labels} labels")
    return f"{attached('image')}, images {images}, labels {labels}, pages {d.get('pagesShown')} of {d.get('pdfPages')}"


if pdfs:
    call("read_file_content", {"path": pdfs[0]["path"]}, lambda d: (
        f"no text layer, {pdf_pages(d)}" if d.get("textLayer") is False else page(d) + f", pdfPages {d.get('pdfPages')}"
    ), label="pdf")
    call("read_file_content", {"path": pdfs[0]["path"], "page": 1}, pdf_pages, label="pdf pages")
else:
    print("skip  read_file_content (pdf)            no PDF over 1 KB found")
images = (call("search_files", {"query": "*.png", "kind": "file", "pageSize": 25}, label="*.png") or {}).get("files") or []
images = [f for f in images if 0 < (f.get("size") or 0) <= 5 * 1024 * 1024]
if images:
    call("read_file_content", {"path": images[0]["path"]}, lambda d: attached("image"), label="image")
else:
    print("skip  read_file_content (image)          no PNG of 5 MiB or less found")
tree = call("list_drive_tree", {"pageSize": 50}, lambda d: f"rows {len(d['rows'])}, more {'yes' if d.get('nextPageToken') else 'no'}")
if tree and tree.get("nextPageToken"):
    call("list_drive_tree", {"pageSize": 50, "pageToken": tree["nextPageToken"]},
         lambda d: f"rows {len(d['rows'])}", label="next page")

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
            + (page(d) if "content" in d else attached("image") if "image" in d
               else f"saved {'yes' if os.path.isfile(d.get('path') or '') else 'NO'}")
            + f", sha256 {'yes' if digested(d) else 'NO'}"
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
