#!/usr/bin/env python3
"""Build the frozen forensic-search corpus in a new directory."""
import argparse
import subprocess
from pathlib import Path

TOKEN = "MAGIC_TOKEN_XYZ"

parser = argparse.ArgumentParser()
parser.add_argument("directory", type=Path)
args = parser.parse_args()
root = args.directory.resolve()
if root.exists():
    raise SystemExit(f"refuse to reuse fixture directory: {root}")
root.mkdir(parents=True)

def write(name, data):
    path = root / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)

write("src/app.py", f"print('{TOKEN}')\n".encode())
write("secrets.env", f"KEY={TOKEN}\n".encode())
write(".gitignore", b"*.env\n")
write(".hidden.txt", f"{TOKEN}\n".encode())
write("blob.dat", f"head\0{TOKEN}\n".encode())
write("blob_bin.dat", b"head\0\xda" + TOKEN.encode() + b"\x80\n")
write("lower.txt", TOKEN.lower().encode())
write("config_utf16.txt", f"setting = {TOKEN}\n".encode("utf-16le"))
write("history.config", f"KEY={TOKEN}\n".encode())

def git(*items):
    subprocess.run(["git", "-C", str(root), *items], check=True, stdout=subprocess.DEVNULL)

git("init", "-q")
git("-c", "user.name=rf-benchmark", "-c", "user.email=rf@example.invalid", "add", "-A")
git("-c", "user.name=rf-benchmark", "-c", "user.email=rf@example.invalid", "commit", "-q", "-m", "seed")
write("history.config", b"KEY=rotated\n")
git("-c", "user.name=rf-benchmark", "-c", "user.email=rf@example.invalid", "add", "history.config")
git("-c", "user.name=rf-benchmark", "-c", "user.email=rf@example.invalid", "commit", "-q", "-m", "rotate")
print(root)
