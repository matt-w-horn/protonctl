#!/usr/bin/env python3
"""Run every protonctl tool once over MCP, as the Claude app does, against this
Mac's real calendar link, Bridge and Drive. Prints one line per call with its
outcome, time and counts, never message, event or file content or names.

It runs in the privacy mode the server starts in. In aliases mode it also
counts raw values that must not appear (addresses, phone numbers, links,
local paths, raw digests) in every result, checks that every result names
its detectors and every `entities` entry carries a `ref`, and starts a
second server to check that the same search gives the same aliases.

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
import re
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


def start() -> None:
    """Start a server and read its replies; the second run of aliases mode calls it again."""
    global server, lines
    server = subprocess.Popen(
        [BIN, "serve"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
    )
    lines = queue.Queue()
    threading.Thread(target=lambda out=server.stdout, q=lines: [q.put(line) for line in out], daemon=True).start()


server: subprocess.Popen
lines: queue.Queue
start()
last_id = 0
failures: list[str] = []
called: set[str] = set()
blocks: list[dict] = []  # the content blocks of the last call, after its JSON
aliases = False  # set from get_status
last_text = ""  # the JSON text of the last call
# Raw values aliases mode must never return (R13, R16, R17), counted, never shown.
# A local path is this machine's home folder: a Drive path such as /home/x is
# Drive's own, and aliases mode keeps its shape.
RAW = {
    "addresses": re.compile(r"[\w.+-]+@[\w-]+(\.[\w-]+)+"),
    "links": re.compile(r"https?://"),
    "local paths": re.compile(re.escape(os.path.expanduser("~")) + r"(/|\b)"),
    "raw digests": re.compile(r"\b([0-9a-f]{40}|[0-9a-f]{64})\b"),
}
# A phone number: international with a +, North American 3-3-4, or national
# with a leading 0. Dates, times and handles, which are base64url, do not
# match. Only text outside `entities` is searched, as section 7 plans.
PHONE = re.compile(
    r"(?<![\w+-])(\+\d[\d ().-]{6,16}\d|\(?[2-9]\d\d\)?[ .-][2-9]\d\d[ .-]\d{4}|0\d{2,4}[ -]\d{3,4}[ -]\d{3,4})(?![\w-])"
)


def strings(value, skip: str = "") -> list[str]:
    """Every string in a result, keys included, leaving out the field named `skip`."""
    if isinstance(value, dict):
        return [s for k, v in value.items() if k != skip for s in [k, *strings(v)]]
    if isinstance(value, list):
        return [s for v in value for s in strings(v)]
    return [value] if isinstance(value, str) else []


def leak_check(shown: str, text: str, data: dict) -> None:
    """Aliases mode: count raw values, and require `detectors`, a `ref` per entity and no `dropped`."""
    raw = {kind: len(p.findall(text)) for kind, p in RAW.items()}
    raw["phone numbers"] = sum(len(PHONE.findall(s)) for s in strings(data, skip="entities"))
    found = ", ".join(f"{n} {kind}" for kind, n in raw.items() if n)
    if found:
        failures.append(f"{shown}: {found}")
    if "detectors" not in data:
        failures.append(f"{shown}: no detectors")
    without_ref = sum(not (e or {}).get("ref") for e in (data.get("entities") or {}).values())
    if without_ref:
        failures.append(f"{shown}: {without_ref} entities without a ref")
    if data.get("dropped"):
        failures.append(f"{shown}: {len(data['dropped'])} fields without a policy")
    if blocks:
        failures.append(f"{shown}: {len(blocks)} blocks after the JSON")


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


def failure(reply: dict, text: str) -> str:
    """A failed call's code and length, never its text: off-mode errors can
    quote a Drive path or the Drive CLI's stderr."""
    if "error" in reply:
        error = reply["error"]
        code = error.get("code")
        shown = code if isinstance(code, int) else "?"
        return f"rpc error {shown}, message {len(str(error.get('message') or ''))} chars"
    try:
        fault = json.loads(text)
    except ValueError:
        fault = None
    if isinstance(fault, dict) and isinstance(fault.get("error"), str):
        code = fault["error"]
        shown = code if re.fullmatch(r"[a-z_]{1,40}", code) else "?"
        return f"error {shown}, message {len(str(fault.get('message') or ''))} chars"
    return f"error text, {len(text)} chars"


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
        print(f"FAIL  {shown:<34} {took:5.1f}s  {failure(reply, text)}")
        return None
    global last_text
    last_text = text
    data = json.loads(text)
    blocks[:] = (result.get("content") or [])[1:]
    if aliases:
        leak_check(shown, text, data)
    note = describe(data)
    if name in CONTENT_TOOLS and "provenance" not in data:
        failures.append(f"{shown}: no provenance")
        note += "  [no provenance]"
    print(f"ok    {shown:<34} {took:5.1f}s  {note}")
    return data


def handshake() -> dict:
    """Open the MCP session as a host does."""
    init = rpc("initialize", {
        "protocolVersion": "2025-11-25", "capabilities": {},
        "clientInfo": {"name": "live-check", "version": "0"},
    })["result"]
    send({"jsonrpc": "2.0", "method": "notifications/initialized"})
    return init


init = handshake()
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
    """Downloads and attachments carry their digests: hex, or five words in aliases mode."""
    if aliases:
        ok = all(len((d.get(k) or "").split("-")) == 5 for k in ("sha256", "sha1"))
    else:
        ok = len(d.get("sha256") or "") == 64 and len(d.get("sha1") or "") == 40
    if not ok:
        failures.append("a saved file without sha256 and sha1")
    return ok


status = call("get_status", {}, lambda d: (
    f"privacy {d['privacy']['mode']}, "
    f"calendars {len(d['calendars'])}, mail {'set up' if isinstance(d['mail'], dict) else 'not set up'}, "
    f"drive folder {'yes' if isinstance(d['drive'].get('folder'), str) else 'no'}, "
    f"secrets held {len(d['secretsHeld'])}"
))
if status is None:
    sys.exit("get_status failed; if no privacy mode is set, run `protonctl setup privacy` or `--off` first")
aliases = status["privacy"]["mode"] == "aliases"
if aliases:
    # get_status ran before the mode was known; it holds the config path and
    # the Bridge address, so it is checked too.
    leak_check("get_status", last_text, status)
export_folder = None if aliases else (status.get("export") or {}).get("folder") if isinstance(status.get("export"), dict) else None


def target(f: dict) -> dict:
    """How a Drive tool names a file: its path, or in aliases mode its fileId."""
    return {"fileId": f["fileId"]} if aliases else {"path": f["path"]}


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
    one = target(files[0])
    call("get_file_metadata", one, lambda d: f"kind {d['file']['kind']}, size given {d['file']['size'] is not None}")
    first = call("read_file_content", one, page)
    if first and first.get("nextOffset"):
        call("read_file_content", {**one, "offset": first["nextOffset"]}, page, label="next page")
# Through the CLI: into the download folder, or in aliases mode the memory disk (M2.8).
def fetched(d: dict) -> str:
    """A cloud-only text file's content came back, so the fetch worked."""
    if d.get("content") is None:
        failures.append("cloud-only read: no content")
    return page(d)


cloud = [f for f in (texts or {}).get("files", []) if f.get("cloudOnly") and 0 < (f.get("size") or 0) <= 256 * 1024]
if cloud:
    call("read_file_content", target(cloud[0]), fetched, label="cloud-only")
else:
    print("skip  read_file_content (cloud-only)     no cloud-only file found")
if files and not aliases:
    path = files[0]["path"]
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
elif not files:
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


if pdfs and aliases:
    call("read_file_content", target(pdfs[0]), page, label="pdf")
elif pdfs:
    call("read_file_content", {"path": pdfs[0]["path"]}, lambda d: (
        f"no text layer, {pdf_pages(d)}" if d.get("textLayer") is False else page(d) + f", pdfPages {d.get('pdfPages')}"
    ), label="pdf")
    call("read_file_content", {"path": pdfs[0]["path"], "page": 1}, pdf_pages, label="pdf pages")
else:
    print("skip  read_file_content (pdf)            no PDF over 1 KB found")
images = (call("search_files", {"query": "*.png", "kind": "file", "pageSize": 25}, label="*.png") or {}).get("files") or []
images = [f for f in images if 0 < (f.get("size") or 0) <= 5 * 1024 * 1024]
if images and aliases:
    call("read_file_content", target(images[0]), lambda d: f"image {(d.get('image') or {}).get('mimeType')}, no image block", label="image")
elif images:
    call("read_file_content", {"path": images[0]["path"]}, lambda d: attached("image"), label="image")
else:
    print("skip  read_file_content (image)          no PNG of 5 MiB or less found")
tree = call("list_drive_tree", {"pageSize": 50}, lambda d: f"rows {len(d['rows'])}, more {'yes' if d.get('nextPageToken') else 'no'}")
if tree and tree.get("nextPageToken"):
    call("list_drive_tree", {"pageSize": 50, "pageToken": tree["nextPageToken"]},
         lambda d: f"rows {len(d['rows'])}", label="next page")

# Mail: a recent search, one message and its thread, labels, then one attachment.
def row_shape(d: dict) -> str:
    """The row contract: 16-character messageIds (44-character handles in aliases mode), no encryption field."""
    rows = d.get("messages") or []
    length = 44 if aliases else 16
    long_ids = sum(len(r.get("messageId") or "") != length for r in rows)
    with_encryption = sum("encryption" in r for r in rows)
    if long_ids or with_encryption:
        failures.append(f"search rows: {long_ids} messageIds not {length} characters, {with_encryption} with encryption")
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
if aliases:
    # A sender's ref from one result finds their mail again, with no name in
    # the query (R15).
    refs = [e["ref"] for e in ((recent or {}).get("entities") or {}).values()
            if e.get("type") == "email" and "sender" in (e.get("hints") or [])]

    def found_again(d: dict) -> str:
        if not d.get("messages"):
            failures.append("from:ref: found no message from a sender of a recent row")
        return row_shape(d)

    if refs:
        call("search_threads", {"query": f"from:ref:{refs[0]} newer_than:30d", "pageSize": 5}, found_again, label="from:ref:")
    else:
        print("skip  search_threads (from:ref:)          no sender's address in the recent rows")
# The search the second server run repeats: no mail from today, so none that
# arrives during the run changes its rows.
SAME = {"query": "older_than:1d newer_than:30d", "pageSize": 10}
first_run = call("search_threads", SAME, label="for the second run") if aliases else None
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
            + (page(d) if "content" in d
               else f"image, {'no image block' if aliases else attached('image')}" if "image" in d
               else f"saved {'yes' if os.path.isfile(d.get('path') or '') else 'NO'}")
            + f", sha256 {'yes' if digested(d) else 'NO'}"
        ))
        break

if export_folder:
    made = call("export_drive_manifest", {}, lambda d: f"rows {d['rows']}, complete {d['complete']}, sha256 {'yes' if d.get('sha256') else 'NO'}")
    if made and made["manifest"].startswith(export_folder):
        os.remove(made["manifest"])  # the check removes what it wrote
elif not aliases:
    print("skip  export_drive_manifest                 no [export] folder configured")
untried = sorted(tools - called)
print(f"\ncalled {len(called)} of {len(tools)} tools" + (f"; not called: {', '.join(untried)}" if untried else ""))


def stop() -> None:
    """Stop the way a host does, then check the server cleaned up (R10)."""
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


def typed_aliases(d: dict) -> dict:
    """Each alias in the `entities` table, with its type."""
    return {alias: e.get("type") for alias, e in (d.get("entities") or {}).items()}


stop()
if first_run is not None:
    # A second server run gives the same aliases for the same search (RFC
    # section 7): aliases come from the stored key, not from the process.
    def same_aliases(d: dict) -> str:
        before, after = typed_aliases(first_run), typed_aliases(d)
        differ = sum(before.get(a) != after.get(a) for a in before.keys() | after.keys())
        if differ:
            failures.append(f"second run: {differ} aliases differ")
        return f"aliases {len(after)}, same as the first run {'yes' if before == after else 'NO'}"

    if typed_aliases(first_run):
        print()
        start()
        handshake()
        call("search_threads", SAME, same_aliases, label="second run")
        stop()
    else:
        print("skip  search_threads (second run)        no aliases in the first run's search")
print("all calls passed" if not failures else f"{len(failures)} problem(s): {'; '.join(failures)}")
sys.exit(1 if failures else 0)
