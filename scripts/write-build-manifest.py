#!/usr/bin/env python3
"""Write secret-free build provenance, optionally binding packaged/signed HAPs.

PT_LOAD fingerprints include ELF64 header/program metadata and loaded file bytes.
Only e_shoff/e_shentsize/e_shnum/e_shstrndx are normalized: Hvigor may strip
non-loaded sections and update those fields. This verifies loaded segments, not
byte-for-byte identity of the entire compiled and stripped ELF files.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import stat
import struct
import subprocess
import sys
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
LOAD_HASH_FORMAT = 'TinyPaws-ELF64-PT_LOAD-v1'
NATIVE_ABI = 'arm64-v8a'
MAX_NATIVE_BYTES = 512 << 20
MAX_HAP_BYTES = 2 << 30
MAX_HAP_MEMBERS = 10000
MAX_PAGES_INFO_BYTES = 256 << 10
MAX_PROGRAM_HEADERS = 1024
MAX_LOAD_SEGMENTS = 64
CHUNK_BYTES = 1 << 20
# These ELF64 section-table fields do not affect the loaded native image.
SECTION_TABLE_FIELDS = ((40, 48), (58, 64))


class ProvenanceError(ValueError):
    """An allowlisted error that contains no input file contents or pin values."""


def stream_sha(stream, limit):
    digest = hashlib.sha256()
    count = 0
    while True:
        chunk = stream.read(min(CHUNK_BYTES, limit - count + 1))
        if not chunk:
            return digest.hexdigest(), count
        count += len(chunk)
        if count > limit:
            raise ProvenanceError('artifact exceeds its bounded size limit')
        digest.update(chunk)


def sha(path, limit=MAX_HAP_BYTES):
    with Path(path).open('rb') as stream:
        return stream_sha(stream, limit)[0]


def _read_at(stream, offset, count, size):
    if offset < 0 or count < 0 or offset + count > size:
        raise ProvenanceError('ELF range exceeds file size')
    stream.seek(offset)
    value = stream.read(count)
    if len(value) != count:
        raise ProvenanceError('truncated ELF range')
    return value


def _normalize_header_fields(value, offset):
    result = bytearray(value)
    for start, end in SECTION_TABLE_FIELDS:
        left, right = max(offset, start), min(offset + len(result), end)
        if left < right:
            result[left - offset:right - offset] = bytes(right - left)
    return result


def elf_load_fingerprint(stream, size):
    """Hash bounded ELF64 PT_LOAD file bytes and metadata; never return bytes."""
    if not 64 <= size <= MAX_NATIVE_BYTES:
        raise ProvenanceError('native ELF size is outside its supported bound')
    header = _read_at(stream, 0, 64, size)
    if header[:4] != b'\x7fELF' or header[4] != 2 or header[5] not in (1, 2) or header[6] != 1:
        raise ProvenanceError('native library must be ELF64 with a supported byte order')
    order = '<' if header[5] == 1 else '>'
    fields = struct.unpack_from(order + 'HHIQQQIHHHHHH', header, 16)
    phoff, ehsize, phentsize, phnum = fields[4], fields[7], fields[8], fields[9]
    if ehsize != 64 or phentsize != 56 or not 1 <= phnum <= MAX_PROGRAM_HEADERS:
        raise ProvenanceError('unsupported ELF64 program header table')
    table = _read_at(stream, phoff, phnum * phentsize, size)
    loads = []
    for index in range(phnum):
        segment = struct.unpack_from(order + 'IIQQQQQQ', table, index * phentsize)
        kind, flags, offset, vaddr, paddr, filesz, memsz, align = segment
        if kind != 1:
            continue
        if filesz > memsz or offset + filesz > size:
            raise ProvenanceError('invalid ELF64 PT_LOAD file or memory range')
        if align > 1 and (align & (align - 1) or (vaddr - offset) % align):
            raise ProvenanceError('invalid ELF64 PT_LOAD alignment')
        loads.append(segment)
    if not 1 <= len(loads) <= MAX_LOAD_SEGMENTS or sum(s[5] for s in loads) > MAX_NATIVE_BYTES:
        raise ProvenanceError('unsupported ELF64 PT_LOAD segment count or total size')
    digest = hashlib.sha256()
    digest.update(LOAD_HASH_FORMAT.encode('ascii') + b'\0')
    digest.update(_normalize_header_fields(header, 0))
    digest.update(struct.pack('<I', len(loads)))
    for segment in loads:
        digest.update(struct.pack('<IIQQQQQQ', *segment))
        offset, remaining = segment[2], segment[5]
        stream.seek(offset)
        while remaining:
            chunk = stream.read(min(CHUNK_BYTES, remaining))
            if not chunk:
                raise ProvenanceError('truncated ELF64 PT_LOAD contents')
            digest.update(_normalize_header_fields(chunk, offset))
            offset += len(chunk)
            remaining -= len(chunk)
    return {
        'loadSegmentHashFormat': LOAD_HASH_FORMAT,
        'loadSegmentSha256': digest.hexdigest(),
        'loadSegmentCount': len(loads),
        'loadSegmentBytes': sum(s[5] for s in loads),
    }


def native_library_info(path):
    path = Path(path)
    size = path.stat().st_size
    with path.open('rb') as stream:
        loaded = elf_load_fingerprint(stream, size)
    return {'bytes': size, 'sha256': sha(path, MAX_NATIVE_BYTES), **loaded}


def _zip_members(archive):
    members = archive.infolist()
    if len(members) > MAX_HAP_MEMBERS or len({m.filename for m in members}) != len(members):
        raise ProvenanceError('HAP has too many or duplicate ZIP members')
    if sum(m.file_size for m in members) > MAX_HAP_BYTES:
        raise ProvenanceError('HAP expanded payload exceeds its bounded size limit')
    return {m.filename: m for m in members}


def _hap_version(archive, members):
    member = members.get('module.json')
    if member is None or member.file_size > 128 << 10:
        raise ProvenanceError('HAP module manifest is absent or exceeds its bound')
    try:
        with archive.open(member) as stream:
            app = json.loads(stream.read((128 << 10) + 1))['app']
        version = {key: app[key] for key in ('debug', 'versionCode', 'versionName')}
        if type(version['debug']) is not bool or type(version['versionCode']) is not int or not isinstance(version['versionName'], str):
            raise ValueError()
    except (KeyError, TypeError, ValueError):
        raise ProvenanceError('HAP module version fields are invalid') from None
    return version


def _zip_payload_hashes(archive, members):
    hashes = {}
    for name, member in members.items():
        with archive.open(member) as stream:
            digest, count = stream_sha(stream, min(member.file_size, MAX_HAP_BYTES))
        if count != member.file_size:
            raise ProvenanceError('HAP member size differs from its ZIP metadata')
        hashes[name] = (count, digest)
    return hashes



def _signed_pages_metadata(archive, member, hap_bytes):
    # SDK hap-sign-tool PageInfoGenerator.generateBitMap uses four bits per
    # 4-KiB page, sets only the low two bits and serializes LE BitSet longs.
    # These are structure checks, not verification of executable-page meaning.
    maximum = min(MAX_PAGES_INFO_BYTES, ((hap_bytes + 65535) // 65536) * 8)
    file_type = stat.S_IFMT(member.external_attr >> 16)
    if (member.is_dir() or file_type not in (0, stat.S_IFREG)
            or member.flag_bits & 1 or member.compress_type != zipfile.ZIP_STORED
            or member.file_size > maximum or member.file_size % 8):
        raise ProvenanceError('added .pages.info has unsupported container or bitmap structure')
    with archive.open(member) as stream:
        data = stream.read(maximum + 1)
    if len(data) != member.file_size or any(byte & 0xcc for byte in data):
        raise ProvenanceError('added .pages.info has invalid bounded page bitmap structure')
    return {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest(),
            'format': 'hap-sign-tool PageInfoGenerator 4KiB-page nibble bitmap',
            'structureValidated': True,
            'validationScope': 'Container, bounded length and bitmap bits only; executable-page meaning and cryptographic signatures are not verified here.'}


def attach_hap_provenance(manifest, hap_path, signed_hap_path=None):
    """Attach packaged hashes; fail closed on native or signed payload mismatch."""
    hap_path = Path(hap_path)
    if hap_path.stat().st_size > MAX_HAP_BYTES:
        raise ProvenanceError('HAP exceeds its bounded size limit')
    with zipfile.ZipFile(hap_path) as archive:
        members = _zip_members(archive)
        version = _hap_version(archive, members)
        if version['debug'] != (manifest['hapBuildMode'] == 'debug'):
            raise ProvenanceError('HAP debug flag differs from the requested build mode')
        prefix = 'libs/' + NATIVE_ABI + '/'
        packaged = {name[len(prefix):]: item for name, item in members.items()
                    if name.startswith(prefix) and name.endswith('.so') and '/' not in name[len(prefix):]}
        if set(packaged) != set(manifest['nativeLibraries']):
            raise ProvenanceError('HAP native library set differs from compiled native libraries')
        for name, compiled in manifest['nativeLibraries'].items():
            member = packaged[name]
            if member.file_size > MAX_NATIVE_BYTES:
                raise ProvenanceError('HAP native library exceeds its bounded size limit')
            with archive.open(member) as stream:
                packaged_sha, count = stream_sha(stream, member.file_size)
            with archive.open(member) as stream:
                loaded = elf_load_fingerprint(stream, member.file_size)
            if count != member.file_size or loaded['loadSegmentSha256'] != compiled['loadSegmentSha256']:
                raise ProvenanceError('HAP PT_LOAD content differs from compiled native library')
            compiled.update(packagedBytes=count, packagedSha256=packaged_sha,
                            packagedLoadSegmentSha256=loaded['loadSegmentSha256'],
                            loadSegmentsMatchCompiledLibrary=True)
        manifest.update(unsignedHapSha256=sha(hap_path), unsignedHapBytes=hap_path.stat().st_size,
                        unsignedHapVersion=version)
        if signed_hap_path is not None:
            signed_hap_path = Path(signed_hap_path)
            if signed_hap_path.stat().st_size > MAX_HAP_BYTES:
                raise ProvenanceError('signed HAP exceeds its bounded size limit')
            unsigned_payload = _zip_payload_hashes(archive, members)
            with zipfile.ZipFile(signed_hap_path) as signed:
                signed_members = _zip_members(signed)
                signed_version = _hap_version(signed, signed_members)
                added = signed_members.keys() - members.keys()
                if members.keys() - signed_members.keys() or added - {'.pages.info'}:
                    raise ProvenanceError('signed HAP ZIP payload member set differs from packaged unsigned HAP')
                metadata = {}
                if '.pages.info' in added:
                    metadata['.pages.info'] = _signed_pages_metadata(
                        signed, signed_members['.pages.info'], signed_hap_path.stat().st_size)
                original_members = {name: member for name, member in signed_members.items() if name in members}
                same = unsigned_payload == _zip_payload_hashes(signed, original_members)
            if not same:
                raise ProvenanceError('signed HAP ZIP payload differs from packaged unsigned HAP')
            manifest.update(signedHapSha256=sha(signed_hap_path), signedHapBytes=signed_hap_path.stat().st_size,
                            signedHapVersion=signed_version, signedPayloadMatchesPackagedUnsignedPayload=True,
                            signedAddedMetadata=metadata,
                            signedApplicationPayloadComparison='All original ZIP member names and bytes match exactly; only newly added .pages.info is separately recorded.')
    manifest['nativePayloadComparison'] = (
        'ELF64 PT_LOAD file bytes and metadata; section-table header fields normalized; '
        'non-loaded sections may differ after Hvigor strip')
    return manifest


def _command_output(command, root):
    result = subprocess.run(command, cwd=root, timeout=15, check=False,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode:
        raise ProvenanceError('build metadata command failed')
    return result.stdout.decode('utf-8', errors='strict').strip()


def build_manifest(root, hap_mode, native_profile, environment=None):
    environment = os.environ if environment is None else environment
    fingerprint = environment.get('PAWS_ENGINE_BUILD_ID', '')
    if len(fingerprint) != 64 or any(c not in '0123456789abcdef' for c in fingerprint):
        raise ProvenanceError('PAWS_ENGINE_BUILD_ID must be a lowercase SHA256 build fingerprint')
    libraries = {p.name: native_library_info(p) for p in sorted((root / 'entry/libs' / NATIVE_ABI).glob('*.so'))}
    if not libraries:
        raise ProvenanceError('compiled native library directory is empty')
    raw = environment.get('PAWS_RAW_ENGINE_DIAGNOSTIC') == '1'
    manifest = {
        'format': 'TinyPaws-native-build-v1',
        'builtAt': datetime.now(timezone.utc).isoformat(),
        'hapBuildMode': hap_mode,
        'nativeProfile': native_profile,
        'diagnosticBypass': raw,
        'wireCapture': environment.get('PAWS_SOCKET_WIRE_CAPTURE') == '1',
        # This is compile-time configuration evidence; runtime validates the
        # selected node against the pin. No raw pin or node fingerprint escapes.
        'diagnosticPeerPinned': raw and bool(environment.get('PAWS_BENCH_PEER_IP')) and bool(environment.get('PAWS_BENCH_NODE_FINGERPRINT')),
        'diagnosticPeerPinScope': 'compile-time configuration; runtime selected-node validation is separate',
        'buildFingerprint': fingerprint,
        'nativeABI': NATIVE_ABI,
        'nativeLibraries': libraries,
        'rustCompiler': _command_output(['rustc', '--version'], root),
        'sourceCommitAtBuild': _command_output(['git', 'rev-parse', 'HEAD'], root),
        'sourceDirty': bool(_command_output(['git', 'status', '--porcelain=v1', '-z', '--untracked-files=normal'], root)),
    }
    return manifest


def write_manifest(path, manifest):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode='w', encoding='utf-8', dir=path.parent,
                                         prefix='.build-manifest-', delete=False) as stream:
            temporary = Path(stream.name)
            json.dump(manifest, stream, indent=2)
            stream.write('\n')
        os.chmod(temporary, 0o644)
        os.replace(temporary, path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--hap-mode', required=True, choices=('release', 'debug'))
    parser.add_argument('--native-profile', required=True, choices=('release', 'debug'))
    parser.add_argument('--hap-path', type=Path, help='packaged unsigned HAP to bind after Hvigor packaging')
    parser.add_argument('--signed-hap-path', type=Path, help='signed HAP; requires --hap-path to prove payload identity')
    args = parser.parse_args(argv)
    if args.signed_hap_path is not None and args.hap_path is None:
        parser.error('--signed-hap-path requires --hap-path')
    try:
        manifest = build_manifest(ROOT, args.hap_mode, args.native_profile)
        if args.hap_path is not None:
            attach_hap_provenance(manifest, args.hap_path, args.signed_hap_path)
        write_manifest(ROOT / 'dist/Paws-build-manifest.json', manifest)
    except ProvenanceError as error:
        print('build manifest generation failed: ' + str(error), file=sys.stderr)
        return 1
    except (OSError, ValueError, zipfile.BadZipFile, subprocess.SubprocessError) as error:
        # External exception strings can contain paths or file contents.
        print('build manifest generation failed (' + type(error).__name__ + ')', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
