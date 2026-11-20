"""Runs the script of `.github/workflows/upstream-review.yml` against a fake `gh`.

The script is taken from the workflow file as it is, so these tests check the code
that runs on GitHub. The fake `gh` serves canned API responses, applies `--jq`
with the real `jq`, and records the comments the script posts.
"""

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest

WORKFLOW = Path(__file__).resolve().parents[2] / ".github/workflows/upstream-review.yml"
MARKER = re.compile(r"<!-- upstream-head: ([0-9a-f]{40}) -->")

pytestmark = pytest.mark.skipif(
    sys.platform != "linux" or not shutil.which("bash") or not shutil.which("jq"),
    reason="needs bash, GNU date and jq, as on the GitHub runner",
)

FAKE_GH = r"""#!/usr/bin/env python3
import json, os, subprocess, sys

state_dir = os.environ["FAKE_GH_DIR"]
with open(os.path.join(state_dir, "state.json")) as f:
    state = json.load(f)
with open(os.path.join(state_dir, "calls.log"), "a") as f:
    f.write(json.dumps(sys.argv[1:]) + "\n")

def fail(message):
    sys.stderr.write(f"gh: {message}\n")
    sys.exit(1)

args = sys.argv[1:]
if args[:2] == ["label", "create"]:
    sys.exit(0)
if args[:2] == ["issue", "list"]:
    print(state.get("issue") or "")
    sys.exit(0)
if args[:2] == ["issue", "create"]:
    print("https://github.com/ferrofolio/ferrofolio/issues/7")
    sys.exit(0)
if args[:2] == ["issue", "comment"]:
    if state.get("post_error"):
        fail("Server Error (HTTP 502)")
    body = open(args[args.index("--body-file") + 1]).read()
    posted = os.path.join(state_dir, "posted")
    os.makedirs(posted, exist_ok=True)
    with open(os.path.join(posted, f"{len(os.listdir(posted)):04d}.md"), "w") as f:
        f.write(body)
    sys.exit(0)
if args[0] != "api":
    fail(f"unexpected command {args}")

jq = None
path = None
i = 1
while i < len(args):
    if args[i] == "--jq":
        jq = args[i + 1]
        i += 2
        continue
    if not args[i].startswith("-") and path is None:
        path = args[i]
    i += 1

if "/issues/" in path and path.endswith("/comments"):
    if state.get("comments_error"):
        fail("Server Error (HTTP 502)")
    data = state.get("comments", [])
elif "/compare/" in path:
    status = state["compare_status"]
    if status == 404:
        fail("Not Found (HTTP 404)")
    if status == 500:
        fail("Server Error (HTTP 500)")
    data = {"status": status, "commits": state.get("compare_commits", [])}
elif "/commits?" in path:
    data = state.get("recent_commits", [])
elif "/commits/" in path:
    data = {"sha": state["head"]}
else:
    fail(f"unexpected path {path}")

if jq is None:
    print(json.dumps(data))
else:
    result = subprocess.run(["jq", "-r", jq], input=json.dumps(data), text=True,
                            capture_output=True)
    sys.stdout.write(result.stdout)
    sys.stderr.write(result.stderr)
    sys.exit(result.returncode)
"""


def _step():
    """Returns the env and the script of the workflow's only step."""
    lines = WORKFLOW.read_text().splitlines()
    env_at = next(i for i, line in enumerate(lines) if line == "        env:")
    run_at = next(i for i, line in enumerate(lines) if line == "        run: |")
    env = {}
    for line in lines[env_at + 1 : run_at]:
        match = re.fullmatch(r" {10}(\w+): (.*)", line)
        if match:
            env[match[1]] = match[2].strip('"')
    script = textwrap.dedent("\n".join(lines[run_at + 1 :]))
    return env, script


def sha(n):
    return hashlib.sha1(str(n).encode()).hexdigest()


def commit(n, subject=None, author="Upstream Author"):
    return {
        "sha": sha(n),
        "html_url": f"https://github.com/finanze/finanze/commit/{sha(n)}",
        "commit": {
            "message": f"{subject or f'fix: change number {n}'}\n\nBody that is not listed.",
            "author": {"name": author, "date": "2026-11-17T10:00:00Z"},
        },
    }


def bot(body):
    return {"user": {"login": "github-actions[bot]"}, "body": body}


def marker(n):
    return f"<!-- upstream-head: {sha(n)} -->"


class Run:
    def __init__(self, result, directory):
        self.returncode = result.returncode
        self.stdout = result.stdout
        self.stderr = result.stderr
        posted = directory / "posted"
        self.posted = (
            [p.read_text() for p in sorted(posted.iterdir())] if posted.exists() else []
        )
        self.calls = [
            json.loads(line)
            for line in (directory / "calls.log").read_text().splitlines()
        ]


@pytest.fixture
def run_workflow(tmp_path):
    env_values, script = _step()
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    gh = bin_dir / "gh"
    gh.write_text(FAKE_GH.replace("#!/usr/bin/env python3", f"#!{sys.executable}", 1))
    gh.chmod(0o755)

    def run(**state):
        state.setdefault("issue", "84")
        state.setdefault("head", sha(100))
        (tmp_path / "state.json").write_text(json.dumps(state))
        (tmp_path / "calls.log").write_text("")
        shutil.rmtree(tmp_path / "posted", ignore_errors=True)
        runner_temp = tmp_path / "runner"
        shutil.rmtree(runner_temp, ignore_errors=True)
        runner_temp.mkdir()
        env = {
            **env_values,
            "GH_TOKEN": "test",
            "REPO": "ferrofolio/ferrofolio",
            "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
            "RUNNER_TEMP": str(runner_temp),
            "FAKE_GH_DIR": str(tmp_path),
            "HOME": str(tmp_path),
        }
        result = subprocess.run(
            ["bash", "-c", script], env=env, capture_output=True, text=True
        )
        return Run(result, tmp_path)

    return run


def tasks(body):
    return [line for line in body.splitlines() if line.startswith("- [ ] ")]


def test_first_run_creates_the_issue_and_lists_the_lookback_window(run_workflow):
    run = run_workflow(issue=None, recent_commits=[commit(3), commit(2), commit(1)])

    assert run.returncode == 0, run.stderr
    assert any(call[:2] == ["issue", "create"] for call in run.calls)
    [listing] = [
        c for c in run.calls if c[0] == "api" and any("/commits?" in a for a in c)
    ]
    assert any("since=" in arg for arg in listing)
    [body] = run.posted
    assert [t[8:15] for t in tasks(body)] == [sha(1)[:7], sha(2)[:7], sha(3)[:7]]
    assert "the last 7 days" in body
    assert MARKER.findall(body) == [sha(100)]


def test_lists_every_commit_since_the_recorded_one(run_workflow):
    run = run_workflow(
        comments=[
            bot(f"older report\n{marker(10)}"),
            {"user": {"login": "someone"}, "body": f"quoting {marker(99)}"},
            bot(f"latest report\n{marker(50)}"),
        ],
        compare_status="ahead",
        compare_commits=[commit(51), commit(52)],
    )

    assert run.returncode == 0, run.stderr
    [compare, *_] = [
        c for c in run.calls if c[0] == "api" and any("/compare/" in a for a in c)
    ]
    assert any(f"compare/{sha(50)}...{sha(100)}" in arg for arg in compare)
    [body] = run.posted
    assert [t[8:15] for t in tasks(body)] == [sha(51)[:7], sha(52)[:7]]
    assert "fix: change number 51" in body
    assert "Body that is not listed" not in body
    assert MARKER.findall(body) == [sha(100)]


@pytest.mark.parametrize("status", [404, "diverged", "behind"])
def test_falls_back_to_the_lookback_window_when_the_recorded_commit_is_gone(
    run_workflow, status
):
    run = run_workflow(
        comments=[bot(marker(50))],
        compare_status=status,
        recent_commits=[commit(61), commit(60)],
    )

    assert run.returncode == 0, run.stderr
    [body] = run.posted
    assert "is no longer an ancestor of upstream" in body
    assert [t[8:15] for t in tasks(body)] == [sha(60)[:7], sha(61)[:7]]
    assert MARKER.findall(body) == [sha(100)]


def test_posts_nothing_when_upstream_has_no_new_commits(run_workflow):
    run = run_workflow(comments=[bot(marker(100))])

    assert run.returncode == 0, run.stderr
    assert run.posted == []
    assert not any(any("/compare/" in a for a in c) for c in run.calls)


def test_stops_without_a_report_when_the_comments_cannot_be_read(run_workflow):
    run = run_workflow(comments_error=True, recent_commits=[commit(1)])

    assert run.returncode != 0
    assert run.posted == []


def test_stops_without_a_report_when_the_comparison_fails(run_workflow):
    run = run_workflow(
        comments=[bot(marker(50))], compare_status=500, recent_commits=[commit(1)]
    )

    assert run.returncode != 0
    assert run.posted == []
    assert "HTTP 500" in run.stderr


def test_escapes_upstream_text(run_workflow):
    subject = (
        "fix: ping @alice <!-- hidden --> [x](https://evil.example) | `code` (#245)"
    )
    run = run_workflow(
        comments=[bot(marker(50))],
        compare_status="ahead",
        compare_commits=[commit(51, subject=subject, author="@mallory <b>")],
    )

    assert run.returncode == 0, run.stderr
    [task] = tasks(run.posted[0])
    text = task.split(") ", 1)[1]
    for raw in ("@", "<", ">", "`", "|", "](https://evil"):
        assert raw not in text
    assert "&#64;alice" in text and "&#64;mallory" in text
    assert "[&#35;245](https://github.com/finanze/finanze/issues/245)" in text
    assert "hidden" not in run.stdout + run.stderr


def test_splits_long_reports_and_records_the_checkpoint_last(run_workflow):
    commits = [commit(1000 + n, subject="feat: " + "x" * 200) for n in range(400)]
    run = run_workflow(
        comments=[bot(marker(50))], compare_status="ahead", compare_commits=commits
    )

    assert run.returncode == 0, run.stderr
    assert len(run.posted) >= 2
    assert all(len(body) < 65536 for body in run.posted)
    assert [MARKER.findall(body) for body in run.posted] == [[]] * (
        len(run.posted) - 1
    ) + [[sha(100)]]
    listed = [t[8:15] for body in run.posted for t in tasks(body)]
    assert listed == [sha(1000 + n)[:7] for n in range(400)]
    assert "(part 1 of" in run.posted[0]


def test_records_no_checkpoint_when_a_part_fails_to_post(run_workflow):
    run = run_workflow(
        comments=[bot(marker(50))],
        compare_status="ahead",
        compare_commits=[commit(51)],
        post_error=True,
    )

    assert run.returncode != 0
    assert run.posted == []
