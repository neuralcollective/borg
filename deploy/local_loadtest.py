#!/usr/bin/env python3
import argparse
import io
import json
import math
import os
import tempfile
import time
import urllib.parse
import urllib.request
import zipfile


def api_request(base_url, token, method, path, body=None, headers=None):
    url = base_url.rstrip("/") + path
    req_headers = {"Authorization": f"Bearer {token}"}
    if headers:
        req_headers.update(headers)
    data = None
    if body is not None:
        if isinstance(body, (bytes, bytearray)):
            data = body
        else:
            data = json.dumps(body).encode("utf-8")
            req_headers.setdefault("Content-Type", "application/json")
    req = urllib.request.Request(url, data=data, headers=req_headers, method=method)
    with urllib.request.urlopen(req, timeout=120) as resp:
        payload = resp.read()
    if not payload:
        return None
    return json.loads(payload.decode("utf-8"))


def fetch_token(base_url):
    with urllib.request.urlopen(base_url.rstrip("/") + "/api/auth/token", timeout=30) as resp:
        payload = json.loads(resp.read().decode("utf-8"))
    return payload["token"]


def build_document(doc_id):
    jurisdiction = [
        "Delaware Chancery",
        "S.D.N.Y.",
        "E.D. Tex.",
        "California Superior Court",
    ][doc_id % 4]
    matter = f"Acme Holdings v. Beta Systems {doc_id % 17}"
    citation = f"2024 WL {1_000_000 + doc_id}"
    issue_code = f"ISSUE-{doc_id:07d}"
    needle = f"retrieval-marker-{doc_id:07d}"
    title = f"Motion draft and analysis for {matter}"
    body = f"""# {title}

Matter: {matter}
Jurisdiction: {jurisdiction}
Citation Anchor: {citation}
Issue Code: {issue_code}
Needle Phrase: {needle}

Section 1. Background
The dispute concerns a contractual notice failure and a follow-on fiduciary duty claim.

Section 2. Key Authority
The controlling citation for this document is {citation}. Counsel should verify the treatment
of the notice clause and any carve-out for fraud or gross negligence.

Section 3. Drafting Note
When retrieving this document, the system should be able to find it via the marker phrase
{needle}, the issue code {issue_code}, or the citation {citation}.

Section 4. Conclusion
This synthetic document exists to test large-corpus ingestion and retrieval fidelity.
"""
    return {
        "file_name": f"matter_{doc_id:07d}.md",
        "citation": citation,
        "issue_code": issue_code,
        "needle": needle,
        "body": body,
    }


def create_zip_shard(start_id, count):
    handle = tempfile.NamedTemporaryFile(prefix="borg-loadtest-", suffix=".zip", delete=False)
    handle.close()
    with zipfile.ZipFile(handle.name, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        docs = []
        for doc_id in range(start_id, start_id + count):
            doc = build_document(doc_id)
            zf.writestr(doc["file_name"], doc["body"])
            docs.append(doc)
    return handle.name, docs


def upload_zip_shard(base_url, token, project_id, zip_path, chunk_size, privileged=False):
    size = os.path.getsize(zip_path)
    total_chunks = math.ceil(size / chunk_size)
    session = api_request(
        base_url,
        token,
        "POST",
        f"/api/projects/{project_id}/uploads/sessions",
        body={
            "file_name": os.path.basename(zip_path),
            "mime_type": "application/zip",
            "file_size": size,
            "chunk_size": chunk_size,
            "total_chunks": total_chunks,
            "is_zip": True,
            "privileged": privileged,
        },
    )
    session_id = session["session_id"]
    with open(zip_path, "rb") as fh:
        for chunk_index in range(total_chunks):
            payload = fh.read(chunk_size)
            api_request(
                base_url,
                token,
                "PUT",
                f"/api/projects/{project_id}/uploads/sessions/{session_id}/chunks/{chunk_index}",
                body=payload,
                headers={"Content-Type": "application/octet-stream"},
            )
    api_request(
        base_url,
        token,
        "POST",
        f"/api/projects/{project_id}/uploads/sessions/{session_id}/complete",
    )
    return session_id


def wait_for_sessions(base_url, token, project_id, session_ids, timeout_s):
    deadline = time.time() + timeout_s
    pending = set(session_ids)
    while pending and time.time() < deadline:
        for session_id in list(pending):
            status = api_request(
                base_url,
                token,
                "GET",
                f"/api/projects/{project_id}/uploads/sessions/{session_id}",
            )
            if status["session"]["status"] == "done":
                pending.remove(session_id)
            elif status["session"]["status"] == "failed":
                raise RuntimeError(f"upload session {session_id} failed: {status['session']['error']}")
        if pending:
            time.sleep(2)
    if pending:
        raise RuntimeError(f"timed out waiting for upload sessions: {sorted(pending)}")


def wait_for_indexing(base_url, token, project_id, expected_files, timeout_s):
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        payload = api_request(
            base_url,
            token,
            "GET",
            f"/api/projects/{project_id}/files?limit=1",
        )
        summary = payload["summary"]
        if summary["total_files"] >= expected_files and summary["text_files"] >= expected_files:
            return summary
        time.sleep(3)
    raise RuntimeError("timed out waiting for extracted text / search indexing")


def run_queries(base_url, token, project_id, docs, query_count, top_k):
    stride = max(1, len(docs) // max(1, query_count))
    sample = docs[::stride][:query_count]
    hits = 0
    details = []
    for doc in sample:
        params = urllib.parse.urlencode({
            "q": doc["needle"],
            "project_id": project_id,
            "limit": top_k,
        })
        results = api_request(base_url, token, "GET", f"/api/search?{params}")
        found = any(item.get("file_path", "").endswith(doc["file_name"]) for item in results)
        if found:
            hits += 1
        details.append({
            "query": doc["needle"],
            "expected_file": doc["file_name"],
            "found": found,
            "top_hit": results[0]["file_path"] if results else None,
        })
    return {
        "queries": len(sample),
        "hits": hits,
        "recall_at_k": hits / max(1, len(sample)),
        "details": details,
    }


def main():
    parser = argparse.ArgumentParser(description="Local Borg ingest/retrieval load test")
    parser.add_argument("--base-url", default="http://127.0.0.1:3131")
    parser.add_argument("--files", type=int, default=2000)
    parser.add_argument("--files-per-zip", type=int, default=250)
    parser.add_argument("--chunk-size", type=int, default=8 * 1024 * 1024)
    parser.add_argument("--query-count", type=int, default=25)
    parser.add_argument("--top-k", type=int, default=5)
    parser.add_argument("--timeout-s", type=int, default=1800)
    parser.add_argument("--project-name", default=None)
    args = parser.parse_args()

    token = fetch_token(args.base_url)
    project = api_request(
        args.base_url,
        token,
        "POST",
        "/api/projects",
        body={
            "name": args.project_name or f"Local Load Test {int(time.time())}",
            "mode": "legal",
            "client_name": "Acme Holdings",
            "jurisdiction": "Delaware",
            "matter_type": "discovery",
        },
    )
    project_id = project["id"]

    session_ids = []
    all_docs = []
    created_archives = []

    try:
        for start in range(0, args.files, args.files_per_zip):
            shard_count = min(args.files_per_zip, args.files - start)
            zip_path, docs = create_zip_shard(start, shard_count)
            created_archives.append(zip_path)
            all_docs.extend(docs)
            session_id = upload_zip_shard(
                args.base_url,
                token,
                project_id,
                zip_path,
                args.chunk_size,
            )
            session_ids.append(session_id)
            print(f"uploaded shard {len(session_ids)} ({shard_count} docs) as session {session_id}")

        wait_for_sessions(args.base_url, token, project_id, session_ids, args.timeout_s)
        summary = wait_for_indexing(args.base_url, token, project_id, args.files, args.timeout_s)
        metrics = run_queries(
            args.base_url,
            token,
            project_id,
            all_docs,
            args.query_count,
            args.top_k,
        )
        result = {
            "project_id": project_id,
            "files": args.files,
            "summary": summary,
            "retrieval": metrics,
        }
        print(json.dumps(result, indent=2))
    finally:
        for path in created_archives:
            try:
                os.unlink(path)
            except FileNotFoundError:
                pass


if __name__ == "__main__":
    main()
