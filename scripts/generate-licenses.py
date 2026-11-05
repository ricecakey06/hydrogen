"""Collect upstream license and copyright notices for the three dependency locks."""

import base64
from concurrent.futures import ThreadPoolExecutor
import hashlib
from io import BytesIO
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile
import tomllib
from urllib.request import urlopen
from zipfile import ZipFile


ROOT = Path(__file__).resolve().parents[1]
LOCKS = ("package-lock.json", "src-tauri/Cargo.lock", "official-plugins/Cargo.lock")
OUTPUT = ROOT / "licenses/third-party.json"
CACHE = ROOT / ".build/license-cache"
NOTICE = re.compile(r"^(license|licence|copying|copyright|notice|unlicense|authors)([._-]|$)", re.I)
SOURCE_EXTENSIONS = (".rs", ".js", ".ts", ".c", ".h")
COPYRIGHT = re.compile(
    r"^\s*(?://!?|/\*+|\*|#|<!--|;)\s*"
    r"((?:SPDX-FileCopyrightText:|Copyright\s+(?:\(c\)\s*|©\s*)?).+?)\s*(?:\*/|-->)?\s*$",
    re.I,
)


def download(url, key):
    cached = CACHE / key
    if not cached.exists():
        with urlopen(url, timeout=60) as response:
            cached.write_bytes(response.read())
    return cached.read_bytes()


def npm_archive(entry, cache):
    algorithm, digest = entry["integrity"].split("-", 1)
    digest = base64.b64decode(digest).hex()
    local = cache / "_cacache/content-v2" / algorithm / digest[:2] / digest[2:4] / digest[4:]
    data = local.read_bytes() if local.is_file() else download(entry["resolved"], digest)
    if hashlib.new(algorithm, data).hexdigest() != digest:
        raise ValueError(f"Checksum mismatch: {entry['resolved']}")
    return data


def read_package(archive_data, ecosystem):
    notices, source_copyright = {}, set()
    manifest = "Cargo.toml" if ecosystem == "Cargo" else "package.json"
    with tarfile.open(fileobj=BytesIO(archive_data), mode="r|gz") as archive:
        for file in archive:
            parts = PurePosixPath(file.name).parts[1:]
            if not parts or not file.isfile():
                continue
            name = "/".join(parts)
            if name == manifest:
                content = archive.extractfile(file).read().decode("utf-8")
                metadata = tomllib.loads(content)["package"] if ecosystem == "Cargo" else json.loads(content)
            elif (NOTICE.match(parts[-1]) and not name.endswith(SOURCE_EXTENSIONS)
                  and not set(parts[:-1]) & {"test", "tests", "fixtures"}):
                notices[name] = archive.extractfile(file).read().decode("utf-8-sig", errors="replace").strip()
            elif name.endswith(SOURCE_EXTENSIONS):
                with archive.extractfile(file) as source:
                    header = source.read(2048).decode("utf-8", errors="replace")
                for line in header.splitlines()[:24]:
                    if match := COPYRIGHT.match(line):
                        source_copyright.add(match[1])
    license_file = metadata.get("license-file")
    if license_file and license_file not in notices:
        with tarfile.open(fileobj=BytesIO(archive_data)) as archive:
            root = archive.getmembers()[0].name.split("/")[0]
            notices[license_file] = archive.extractfile(f"{root}/{license_file}").read().decode("utf-8-sig").strip()
    return metadata, notices, source_copyright


def upstream_texts(urls):
    for url in urls:
        address, _, member = url.partition("#")
        data = download(address, hashlib.sha256(address.encode()).hexdigest())
        if member:
            with ZipFile(BytesIO(data)) as archive:
                data = archive.read(member)
        yield data.decode("utf-8-sig").replace("\r\n", "\n").strip()


def main():
    CACHE.mkdir(parents=True, exist_ok=True)
    locks = {name: (ROOT / name).read_text(encoding="utf-8") for name in LOCKS}
    npm = "npm.cmd" if os.name == "nt" else "npm"
    npm_cache = Path(subprocess.check_output([npm, "config", "get", "cache"], text=True).strip())
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    cargo_cache = {file.name: file for file in cargo_home.glob("registry/cache/*/*.crate")}
    sources = json.loads((ROOT / "licenses/sources.json").read_text(encoding="utf-8"))
    entries = {}
    for path, entry in json.loads(locks[LOCKS[0]])["packages"].items():
        if path:
            entries["npm", path.rsplit("node_modules/", 1)[1], entry["version"]] = entry
    for lock in LOCKS[1:]:
        for entry in tomllib.loads(locks[lock])["package"]:
            if "source" in entry:
                entries["Cargo", entry["name"], entry["version"]] = entry

    def collect(key):
        ecosystem, name, version = key
        entry = entries[key]
        if ecosystem == "Cargo":
            filename = f"{name}-{version}.crate"
            if entry["source"] != "registry+https://github.com/rust-lang/crates.io-index":
                raise ValueError(f"Unsupported source for {name}: {entry['source']}")
            data = cargo_cache[filename].read_bytes()
            if hashlib.sha256(data).hexdigest() != entry["checksum"]:
                raise ValueError(f"Checksum mismatch: {filename}")
        else:
            data = npm_archive(entry, npm_cache)
        return read_package(data, ecosystem)

    with ThreadPoolExecutor(max_workers=8) as pool:
        collected = dict(zip(entries, pool.map(collect, entries)))
    texts, packages, missing = {}, [], []

    def add_package(ecosystem, name, version, license, content):
        indexes = [texts.setdefault(text, len(texts)) for text in content if text]
        packages.append({"name": name, "version": version, "ecosystem": ecosystem,
                         "license": license, "texts": list(dict.fromkeys(indexes))})

    for ecosystem, name, version in sorted(entries, key=lambda key: (key[1].lower(), key[0], key[2])):
        metadata, files, headers = collected[ecosystem, name, version]
        prefix = next((prefix for prefix in ("@rolldown/binding-", "@tauri-apps/cli-") if name.startswith(prefix)), None)
        if not files and ecosystem == "npm" and prefix:
            parent = {"@rolldown/binding-": "rolldown", "@tauri-apps/cli-": "@tauri-apps/cli"}[prefix]
            files = collected[ecosystem, parent, version][1]
        key = f"{ecosystem}:{name}@{version}"
        upstream = list(upstream_texts(sources["supplements"].get(key, [])))
        if not upstream and not any(NOTICE.match(path.split("/")[-1]) and
                                    not path.split("/")[-1].lower().startswith(("notice", "authors", "copyright"))
                                    for path in files):
            missing.append(key)
        content = [*dict(sorted(files.items())).values(), *upstream]
        additional = [header for header in sorted(headers) if not any(header in text for text in content)]
        if additional:
            content.append("Copyright notices in upstream source files\n\n" + "\n".join(additional))
        license = metadata.get("license") or metadata.get("license-file")
        if not license:
            raise ValueError(f"No license declaration: {key}")
        add_package(ecosystem, name, version, license, content)
    if missing:
        raise SystemExit("Provide upstream license files in licenses/sources.json for:\n" + "\n".join(missing))
    for item in sources["bundled"]:
        add_package("Native", item["name"], item["version"], item["license"], upstream_texts(item["files"]))
    packages.sort(key=lambda item: (item["name"].lower(), item["ecosystem"], item["version"]))
    result = {"locks": {name: hashlib.sha256(data.encode()).hexdigest() for name, data in locks.items()},
              "packages": packages, "texts": list(texts)}
    OUTPUT.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(f"Collected {len(packages)} locked dependencies and {len(texts)} unique notices.")


if __name__ == "__main__":
    main()
