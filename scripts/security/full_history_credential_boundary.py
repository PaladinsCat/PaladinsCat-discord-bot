"""Bounded full-history path and known-credential checks using batched Git reads."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
import threading
import time

PATTERN = re.compile(rb"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----|github_pat_[A-Za-z0-9_]{20,}|gh[pousr]_[A-Za-z0-9]{30,}|AKIA[0-9A-Z]{16}|xox[baprs]-[A-Za-z0-9-]{20,}|sk-[A-Za-z0-9]{32,}", re.I)
TEXT = re.compile(r"\.(?:cfg|conf|env|ini|json|md|mjs|ps1|py|rs|sh|toml|ts|tsx|txt|yaml|yml)$", re.I)
OBJECT = re.compile(r"^[0-9a-f]{40}$")
MAX_METADATA = 64 * 1024 * 1024
MAX_CONTENT = 1024 * 1024 * 1024


def path_gate(path: str) -> None:
    normalized = path.replace("\\", "/")
    template = re.search(r"(^|/)(?:secrets|env)/(?:README\.md|\.gitkeep)$|(^|/)\.env\.example$", normalized)
    if not template and re.search(r"(^|/)(?:secrets|env|local)/|(^|/)\.env(?:\.|$)|\.(?:key|pem|p12|pfx|jks)$", normalized, re.I):
        raise ValueError("credential-bearing historical path: " + normalized)


def credential_in_blob(stream, length: int, chunk_size=65536) -> bool:
    tail = b""
    found = False
    while length:
        part = stream.read(min(chunk_size, length))
        if not part:
            raise ValueError("truncated Git object")
        length -= len(part)
        content = tail + part
        found = found or PATTERN.search(content) is not None
        tail = content[-128:]
    return found


def scan(repository: Path) -> dict:
    started = time.monotonic()
    deadline = started + 165
    command = ["git", "--no-pager", "-C", str(repository), "-c", "core.quotepath=false"]
    def git(*args, data=None):
        remaining = deadline - time.monotonic()
        if remaining <= 0: raise TimeoutError("credential scan deadline")
        result = subprocess.run([*command, *args], input=data, capture_output=True, timeout=min(30, remaining),
                                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        if result.returncode or len(result.stdout) > MAX_METADATA:
            raise ValueError("Git metadata check failed or exceeded bound")
        return result.stdout
    if git("rev-parse", "--is-shallow-repository").strip() != b"false":
        raise ValueError("full-history check requires complete Git history")
    paths = git("log", "--all", "--name-only", "--format=", "-z")
    for raw in paths.split(b"\0"):
        name = raw.decode("utf-8", errors="strict").strip("\n")
        if name: path_gate(name)
    candidates = {}
    for raw in git("rev-list", "--objects", "--all").decode("utf-8", errors="strict").splitlines():
        oid, _, name = raw.partition(" ")
        if name.startswith('"'):
            try: name = json.loads(name)
            except ValueError: raise ValueError("unsupported quoted Git path") from None
        if not OBJECT.fullmatch(oid): raise ValueError("invalid Git object inventory")
        if TEXT.search(name): candidates[oid] = name
    if len(candidates) > 500000: raise ValueError("Git inventory exceeds bound")
    metadata = git("cat-file", "--batch-check=%(objectname) %(objecttype) %(objectsize)", data="".join(oid + "\n" for oid in candidates).encode())
    selected = []
    total = 0
    for expected, line in zip(candidates, metadata.decode().splitlines(), strict=True):
        oid, kind, size = line.split(" ")
        if oid != expected or not size.isdecimal(): raise ValueError("Git object metadata mismatch")
        if kind != "blob": continue
        total += int(size)
        if total > MAX_CONTENT: raise ValueError("Git source history exceeds one GiB scan budget")
        selected.append((oid, int(size)))
    process = subprocess.Popen([*command, "cat-file", "--batch"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    def expire():
        if process.poll() is None: process.kill()
    watchdog = threading.Timer(max(0.001, deadline - time.monotonic()), expire)
    watchdog.daemon = True
    watchdog.start()
    try:
        for oid, size in selected:
            if time.monotonic() >= deadline: raise TimeoutError("credential scan deadline")
            process.stdin.write((oid + "\n").encode()); process.stdin.flush()
            header = process.stdout.readline(256).decode().strip()
            if header != f"{oid} blob {size}": raise ValueError("Git object body mismatch")
            found = credential_in_blob(process.stdout, size)
            if process.stdout.read(1) != b"\n": raise ValueError("Git object separator mismatch")
            if found: raise ValueError("credential-like historical content: " + candidates[oid] + " object=" + oid)
        process.stdin.close()
        if process.wait(timeout=5): raise ValueError("Git batch read failed")
    finally:
        watchdog.cancel()
        if process.poll() is None:
            process.kill(); process.wait(timeout=5)
        process.stdout.close()
        if not process.stdin.closed: process.stdin.close()
    return {"status": "FULL_HISTORY_CREDENTIAL_BOUNDARY=PASS", "textObjects": len(selected), "bytes": total,
            "seconds": round(time.monotonic() - started, 3)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(scan(args.repository.resolve())))


if __name__ == "__main__": main()
