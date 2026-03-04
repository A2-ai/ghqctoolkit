import sys
import re


def verify_tag(git_tag):
    with open("Cargo.toml") as f:
        content = f.read()

    match = re.search(r'^\s*version\s*=\s*"([^"]+)"', content, re.MULTILINE)
    if not match:
        print("Could not find version in Cargo.toml")
        return 1

    cargo_version = f"v{match.group(1)}"
    if git_tag != cargo_version:
        print(f"Different version compared to tag: tag={git_tag} cargo={cargo_version}")
        return 1

    return 0


if __name__ == "__main__":
    exit(verify_tag(sys.argv[1]))