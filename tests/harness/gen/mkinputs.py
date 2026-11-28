# mkinputs.py OUT_DIR [--platform linux|windows|macos]: writes the input of
# every case to OUT_DIR/CASE/, and next to it, as expected-INPUT, the file its
# save must give when the case names one. For a test machine without Python:
# run.ps1 -Inputs OUT_DIR.
# A case whose `platforms` does not list the platform is left out; the
# platform is the system this script runs on unless --platform names another.
# Stops when a file does not have the sha256 its case gives.
import argparse
import hashlib
import pathlib
import re
import subprocess
import sys

GEN = pathlib.Path(__file__).resolve().parent
BLANK = re.compile(r'^\s*(#.*)?$')
PAIR = re.compile(r'''^([a-z0-9_]+)\s*=\s*(?:"(.*)"|'(.*)')\s*$''')


def read_case(path):
    case = {}
    for line in path.read_text(encoding='utf-8').splitlines():
        if BLANK.match(line):
            continue
        pair = PAIR.match(line)
        if not pair:
            sys.exit(f'{path}: cannot read line: {line}')
        case[pair.group(1)] = pair.group(2) if pair.group(2) is not None else pair.group(3)
    return case


def generate(spec, out, want):
    script, *args = spec.split()
    subprocess.run([sys.executable, '-I', str(GEN / script), str(out), *args], check=True)
    got = hashlib.sha256(out.read_bytes()).hexdigest()
    if want and got != want:
        sys.exit(f'{out}: sha256 {got}, the case gives {want}')


def this_platform():
    if sys.platform == 'win32':
        return 'windows'
    if sys.platform == 'darwin':
        return 'macos'
    return 'linux'


# A case without `platforms` is for every system.
def runs_on(case, platform):
    platforms = case.get('platforms', '').split()
    return not platforms or platform in platforms


parser = argparse.ArgumentParser()
parser.add_argument('out_dir', type=pathlib.Path)
parser.add_argument('--platform', choices=['linux', 'windows', 'macos'], default=this_platform())
options = parser.parse_args()

out_dir = options.out_dir
cases = sorted((GEN.parent / 'cases').glob('*.toml'))
written = 0
for path in cases:
    case = read_case(path)
    if not runs_on(case, options.platform):
        continue
    written += 1
    folder = out_dir / path.stem
    folder.mkdir(parents=True, exist_ok=True)
    generate(case['generate'], folder / case['input'], case.get('input_sha256'))
    if 'expected_generate' in case:
        generate(case['expected_generate'], folder / ('expected-' + case['input']), case.get('expected_sha256'))
print(f'wrote the inputs of {written} cases to {out_dir}')
if written < len(cases):
    print(f'left out {len(cases) - written} cases that are not for {options.platform}')
