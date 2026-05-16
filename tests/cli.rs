use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{Duration, SecondsFormat, Utc};
use serde_json::Value;
use tempfile::TempDir;

const FIXTURE: &str = "tests/fixtures/basic-repo";
const GOLDEN: &str = "tests/golden";

struct Repo {
    _tmp: TempDir,
    root: PathBuf,
}

impl Repo {
    fn new() -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path().join("repo");
        copy_dir(Path::new(FIXTURE), &root);
        Self { _tmp: tmp, root }
    }

    fn run(&self, args: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_orchid"))
            .arg("--root")
            .arg(&self.root)
            .args(args)
            .output()
            .expect("run orchid");
        assert!(
            output.status.success(),
            "orchid failed\nstdout:{}\nstderr:{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("json stdout")
    }

    fn run_from_cwd(&self, args: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_orchid"))
            .current_dir(&self.root)
            .args(args)
            .output()
            .expect("run orchid");
        assert!(
            output.status.success(),
            "orchid failed\nstdout:{}\nstderr:{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("json stdout")
    }

    fn run_fail(&self, args: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_orchid"))
            .arg("--root")
            .arg(&self.root)
            .args(args)
            .output()
            .expect("run orchid");
        assert!(
            !output.status.success(),
            "orchid unexpectedly passed\nstdout:{}",
            String::from_utf8_lossy(&output.stdout)
        );
        serde_json::from_slice(&output.stdout).expect("json stdout")
    }

    fn run_help(&self, args: &[&str]) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_orchid"))
            .args(args)
            .arg("--help")
            .output()
            .expect("run orchid help");
        assert!(
            output.status.success(),
            "help failed\nstdout:{}\nstderr:{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf8 help")
    }

    fn init_git(&self) {
        git(&self.root, &["init", "-b", "main"]);
        git(&self.root, &["config", "user.email", "test@example.com"]);
        git(&self.root, &["config", "user.name", "Test"]);
        git(&self.root, &["add", "."]);
        git(&self.root, &["commit", "-m", "initial"]);
    }

    fn write_task_file(&self, spec: &str, task_id: &str, status: &str, scope: &str) -> PathBuf {
        let spec_dir = self.root.join("specs").join(spec);
        let task_dir = spec_dir.join("tasks");
        fs::create_dir_all(&task_dir).expect("task dir");
        fs::write(spec_dir.join("requirements.md"), "# Requirements\n").expect("requirements");
        fs::write(spec_dir.join("design.md"), "# Design\n").expect("design");
        let path = task_dir.join(format!("{task_id}.md"));
        fs::write(
            &path,
            format!(
                "+++\nid = \"{task_id}\"\ntitle = \"{task_id}\"\nstatus = \"{status}\"\nscope = [\"{scope}\"]\ndepends = []\ncovers = []\nverification_mode = \"mayor\"\nverification_status = \"pending\"\n+++\n\n## Context\n"
            ),
        )
        .expect("write task");
        path
    }
}

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("create dst");
    for entry in fs::read_dir(src).expect("read src") {
        let entry = entry.expect("dir entry");
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir(&src_path, &dst_path);
        } else {
            fs::copy(&src_path, &dst_path).expect("copy file");
        }
    }
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:{}\nstderr:{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn task_status(root: &Path, path: &str) -> String {
    let text = fs::read_to_string(root.join(path)).expect("task file");
    let start = "+++\n".len();
    let end = text[start..]
        .find("\n+++\n")
        .map(|idx| idx + start)
        .expect("frontmatter end");
    let meta: toml::Value = toml::from_str(&text[start..end]).expect("frontmatter toml");
    meta.get("status")
        .and_then(toml::Value::as_str)
        .unwrap_or("todo")
        .to_string()
}

fn normalized_contract(mut value: Value) -> Value {
    normalize_value(&mut value);
    value
}

fn assert_golden_contract(actual: Value, fixture: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(GOLDEN)
        .join(fixture);
    let expected: Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("golden contract fixture"))
            .expect("golden contract json");
    assert_eq!(
        normalized_contract(actual),
        expected,
        "golden fixture: {fixture}"
    );
}

fn normalize_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, item) in map {
                match key.as_str() {
                    "started_at" | "heartbeat_at" | "released_at" | "completed_at"
                    | "blocked_at" => *item = Value::String("<timestamp>".to_string()),
                    "age_seconds" | "heartbeat_age_seconds" => {
                        *item = Value::String("<age>".to_string())
                    }
                    "lease_id"
                        if item
                            .as_str()
                            .is_some_and(|raw| raw.starts_with("l_") && raw.len() == 14) =>
                    {
                        *item = Value::String("<lease_id>".to_string())
                    }
                    _ => normalize_value(item),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_value(item);
            }
        }
        _ => {}
    }
}

#[test]
fn canonical_binary_json_contracts_are_stable() {
    let repo = Repo::new();

    assert_golden_contract(
        repo.run(&["ready", "--spec", "example", "--explain"]),
        "ready_success.json",
    );

    assert_golden_contract(repo.run_fail(&["ready"]), "ready_scope_required.json");

    assert_golden_contract(
        repo.run(&[
            "lease",
            "example",
            "T001",
            "--owner",
            "worker:agent_123",
            "--lease-id",
            "l_contract",
        ]),
        "lease_success.json",
    );
}

#[test]
fn ready_requires_scope_and_reports_blocked_tasks() {
    let repo = Repo::new();
    let payload = repo.run_from_cwd(&["ready", "--spec", "example"]);
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["ready"][0]["task"], "example/T001");

    let payload = repo.run(&["ready", "--spec", "example", "--explain"]);
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["runtime"]["active_count"], 0);
    assert_eq!(payload["ready"][0]["task"], "example/T001");
    assert_eq!(payload["blocked"][0]["reason"], "unmet dependency:T001");
    assert_eq!(payload["blocked"][1]["reason"], "status:done");

    let payload = repo.run_fail(&["ready"]);
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"], "ready requires --spec or --all-open");
    assert_eq!(payload["code"], "scope_required");
}

#[test]
fn all_open_selects_first_open_numerical_spec_and_skips_inactive() {
    let repo = Repo::new();
    repo.write_task_file("00-done", "T001", "done", "src/done/");
    repo.write_task_file("01-first", "T001", "todo", "src/first/");
    repo.write_task_file("02-second", "T001", "todo", "src/second/");
    repo.write_task_file("DRAFT-00-draft", "T001", "todo", "src/draft/");
    repo.write_task_file("TBD-00-tbd", "T001", "todo", "src/tbd/");
    repo.write_task_file("DONE-99-closed", "T001", "todo", "src/closed/");

    let payload = repo.run(&["ready", "--all-open", "--explain"]);
    assert_eq!(payload["selected_specs"], serde_json::json!(["01-first"]));
    assert_eq!(payload["ready"][0]["task"], "01-first/T001");
    assert_eq!(
        payload["skipped_inactive_specs"],
        serde_json::json!(["DONE-99-closed", "DRAFT-00-draft", "TBD-00-tbd"])
    );

    let payload = repo.run_fail(&[
        "lease",
        "DONE-99-closed",
        "T001",
        "--owner",
        "worker:agent_123",
    ]);
    assert_eq!(payload["code"], "inactive_spec");
}

#[test]
fn lease_runtime_and_parallel_guards_match_python_contract() {
    let repo = Repo::new();
    let payload = repo.run(&[
        "lease",
        "example",
        "T001",
        "--owner",
        "worker:agent_123",
        "--lease-id",
        "l_one",
    ]);
    assert_eq!(payload["lease_id"], "l_one");
    assert_eq!(payload["lease_mode"], "single");
    assert_eq!(payload["runtime"]["active_count"], 1);
    assert!(repo.root.join(".orchid/leases/l_one.json").exists());
    assert!(!repo.root.join(".orch").exists());
    let running = repo.run_from_cwd(&["running"]);
    assert_eq!(running["leases"][0]["lease_id"], "l_one");
    assert_eq!(
        task_status(&repo.root, "specs/example/tasks/T001.md"),
        "todo"
    );

    repo.write_task_file("example", "T005", "todo", "src/other/");
    let payload = repo.run_fail(&[
        "lease",
        "example",
        "T005",
        "--owner",
        "worker:agent_456",
        "--lease-id",
        "l_two",
    ]);
    assert_eq!(payload["code"], "parallel_not_confirmed");

    let payload = repo.run(&[
        "lease",
        "example",
        "T005",
        "--owner",
        "worker:agent_456",
        "--lease-id",
        "l_two",
        "--allow-parallel",
    ]);
    assert_eq!(payload["lease_mode"], "parallel");
    assert_eq!(payload["runtime"]["active_count"], 2);
}

#[test]
fn serial_and_scope_conflicts_are_rejected() {
    let repo = Repo::new();
    repo.run(&[
        "lease",
        "example",
        "T001",
        "--owner",
        "worker:agent_123",
        "--lease-id",
        "l_test",
    ]);
    repo.write_task_file("example", "T005", "todo", "src/other/");
    let payload = repo.run_fail(&[
        "lease",
        "example",
        "T005",
        "--owner",
        "worker:agent_456",
        "--serial",
    ]);
    assert_eq!(payload["code"], "serial_blocked");

    let path = repo.root.join("specs/example/tasks/T004.md");
    fs::write(
        path,
        "+++\nid = \"T004\"\ntitle = \"Overlap\"\nstatus = \"todo\"\nscope = [\"src/feature/file.rs\"]\ndepends = []\ncovers = []\nverification_mode = \"mayor\"\nverification_status = \"pending\"\n+++\n\n## Context\n",
    )
    .expect("write overlap task");
    let payload = repo.run_fail(&["lease", "example", "T004", "--owner", "worker:agent_456"]);
    assert_eq!(payload["code"], "scope_conflict");
}

#[test]
fn next_moves_through_dispatch_wait_validate_and_recover() {
    let repo = Repo::new();
    let payload = repo.run(&["next", "--spec", "example", "--explain"]);
    assert_eq!(payload["phase"], "dispatch");
    assert!(payload["recommended_action"]
        .as_str()
        .unwrap()
        .contains("lease example T001"));

    let lease = repo.run(&[
        "lease",
        "example",
        "T001",
        "--owner",
        "worker:agent_123",
        "--lease-id",
        "l_test",
    ]);
    let payload = repo.run(&["next", "--spec", "example"]);
    assert_eq!(payload["phase"], "wait");

    fs::write(
        repo.root.join(lease["report"].as_str().unwrap()),
        "+++\nlease_id = \"l_test\"\nstatus = \"ready_for_validation\"\ncommands_run = []\nresult = \"passed\"\n+++\n\n## Summary\n",
    )
    .expect("report");
    let payload = repo.run(&["next", "--spec", "example"]);
    assert_eq!(payload["phase"], "validate");
    assert_eq!(payload["reports_ready"][0]["lease_id"], "l_test");

    let repo = Repo::new();
    repo.run(&[
        "lease",
        "example",
        "T001",
        "--owner",
        "worker:agent_123",
        "--lease-id",
        "l_test",
    ]);
    let lease_path = repo.root.join(".orchid/leases/l_test.json");
    let mut lease: Value =
        serde_json::from_str(&fs::read_to_string(&lease_path).expect("lease json")).unwrap();
    let old = (Utc::now() - Duration::hours(2)).to_rfc3339_opts(SecondsFormat::Secs, false);
    lease["started_at"] = Value::String(old.clone());
    lease["heartbeat_at"] = Value::String(old);
    fs::write(&lease_path, serde_json::to_string(&lease).unwrap()).expect("rewrite lease");
    let payload = repo.run(&["next", "--spec", "example", "--older-than", "30m"]);
    assert_eq!(payload["phase"], "recover");
    assert_eq!(payload["stale"][0]["lease_id"], "l_test");
}

#[test]
fn complete_updates_only_task_and_next_finds_stage_or_cleanup() {
    let repo = Repo::new();
    repo.run(&[
        "lease",
        "example",
        "T001",
        "--owner",
        "worker:agent_123",
        "--lease-id",
        "l_test",
    ]);
    let payload = repo.run(&[
        "complete",
        "--lease",
        "l_test",
        "--verified-by",
        "validator:agent_456",
    ]);
    assert_eq!(payload["status"], "done");
    assert_eq!(payload["runtime"]["active_count"], 0);
    assert_eq!(
        task_status(&repo.root, "specs/example/tasks/T001.md"),
        "done"
    );
    assert_eq!(
        task_status(&repo.root, "specs/example/tasks/T002.md"),
        "todo"
    );

    let payload = repo.run(&["next", "--spec", "example"]);
    assert_eq!(payload["phase"], "cleanup");

    let repo = Repo::new();
    repo.init_git();
    repo.run(&[
        "lease",
        "example",
        "T001",
        "--owner",
        "worker:agent_123",
        "--lease-id",
        "l_test",
    ]);
    fs::write(
        repo.root.join("src/feature/work.txt"),
        "changed during lease\n",
    )
    .unwrap();
    repo.run(&[
        "complete",
        "--lease",
        "l_test",
        "--verified-by",
        "validator:agent_456",
    ]);
    let payload = repo.run(&["next", "--spec", "example"]);
    assert_eq!(payload["phase"], "stage");
    assert_eq!(payload["stage"][0]["lease_id"], "l_test");
    assert!(payload["stage"][0]["pathspecs"]
        .as_array()
        .unwrap()
        .contains(&Value::String("src/feature/work.txt".to_string())));
}

#[test]
fn complete_writes_task_arrays_in_multiline_style() {
    let repo = Repo::new();
    let task_path = repo.root.join("specs/example/tasks/T001.md");
    let original = fs::read_to_string(&task_path).expect("task file");
    let multiline = original
        .replace(
            "scope = [\"src/feature/\"]",
            "scope = [\n    \"src/feature/\",\n]",
        )
        .replace("covers = [\"R001\"]", "covers = [\n    \"R001\",\n]");
    fs::write(&task_path, multiline).expect("rewrite task file");

    repo.run(&[
        "lease",
        "example",
        "T001",
        "--owner",
        "worker:agent_123",
        "--lease-id",
        "l_test",
    ]);
    repo.run(&[
        "complete",
        "--lease",
        "l_test",
        "--verified-by",
        "validator:agent_456",
    ]);

    let rewritten = fs::read_to_string(&task_path).expect("task file");
    assert!(rewritten.contains("scope = [\n    \"src/feature/\",\n]"));
    assert!(rewritten.contains("covers = [\n    \"R001\",\n]"));
    assert!(!rewritten.contains("scope = [\"src/feature/\"]"));
    assert!(!rewritten.contains("covers = [\"R001\"]"));
}

#[test]
fn block_writes_task_arrays_in_multiline_style() {
    let repo = Repo::new();
    let task_path = repo.root.join("specs/example/tasks/T002.md");
    let original = fs::read_to_string(&task_path).expect("task file");
    let multiline = original
        .replace(
            "scope = [\"src/dependent/\"]",
            "scope = [\n    \"src/dependent/\",\n]",
        )
        .replace("depends = [\"T001\"]", "depends = [\n    \"T001\",\n]");
    fs::write(&task_path, multiline).expect("rewrite task file");

    repo.run(&[
        "block",
        "example",
        "T002",
        "--reason",
        "waiting for external input",
    ]);

    let rewritten = fs::read_to_string(&task_path).expect("task file");
    assert!(rewritten.contains("scope = [\n    \"src/dependent/\",\n]"));
    assert!(rewritten.contains("depends = [\n    \"T001\",\n]"));
    assert!(!rewritten.contains("scope = [\"src/dependent/\"]"));
    assert!(!rewritten.contains("depends = [\"T001\"]"));
}

#[test]
fn git_touched_and_stage_plan_split_scope_and_baseline() {
    let repo = Repo::new();
    repo.init_git();
    fs::write(
        repo.root.join("src/other/preexisting.txt"),
        "dirty before lease\n",
    )
    .unwrap();
    repo.run(&[
        "lease",
        "example",
        "T001",
        "--owner",
        "worker:agent_123",
        "--lease-id",
        "l_test",
    ]);
    fs::write(
        repo.root.join("src/feature/work.txt"),
        "changed during lease\n",
    )
    .unwrap();
    fs::write(
        repo.root.join("src/other/work.txt"),
        "changed during lease\n",
    )
    .unwrap();
    let payload = repo.run(&["git-touched", "--lease", "l_test"]);
    assert_eq!(
        payload["touched_in_scope"],
        serde_json::json!(["src/feature/work.txt"])
    );
    assert_eq!(
        payload["out_of_scope"],
        serde_json::json!(["src/other/work.txt"])
    );
    assert_eq!(
        payload["preexisting_dirty"],
        serde_json::json!(["src/other/preexisting.txt"])
    );
    assert_eq!(payload["safe_to_stage"], false);

    let repo = Repo::new();
    repo.init_git();
    repo.run(&[
        "lease",
        "example",
        "T001",
        "--owner",
        "worker:agent_123",
        "--lease-id",
        "l_test",
    ]);
    fs::write(
        repo.root.join("src/feature/work.txt"),
        "changed during lease\n",
    )
    .unwrap();
    let payload = repo.run(&["git-stage-plan", "--lease", "l_test"]);
    assert_eq!(payload["lease_id"], "l_test");
    assert_eq!(payload["task"], "example/T001");
    assert_eq!(
        payload["pathspecs"],
        serde_json::json!(["src/feature/work.txt"])
    );
}

#[test]
fn packet_close_cleanup_and_research_lifecycle() {
    let repo = Repo::new();
    let lease = repo.run(&[
        "lease",
        "example",
        "T001",
        "--owner",
        "worker:agent_123",
        "--lease-id",
        "l_test",
    ]);
    let packet = repo.run(&["packet", "--lease", "l_test", "--role", "worker"]);
    assert_eq!(packet["packet"], ".orchid/packets/l_test-worker.md");
    assert!(
        fs::read_to_string(repo.root.join(packet["packet"].as_str().unwrap()))
            .unwrap()
            .contains("Worker Packet")
    );
    fs::write(
        repo.root.join(lease["report"].as_str().unwrap()),
        "+++\nlease_id = \"l_test\"\nstatus = \"ready_for_validation\"\ncommands_run = []\nresult = \"passed\"\n+++\n\n## Summary\n",
    )
    .unwrap();
    repo.run(&["complete", "--lease", "l_test", "--verified-by", "mayor"]);
    let payload = repo.run(&["close", "--lease", "l_test"]);
    assert!(payload["deleted"]
        .as_array()
        .unwrap()
        .contains(&Value::String(".orchid/leases/l_test.json".to_string())));
    assert!(payload["deleted"]
        .as_array()
        .unwrap()
        .contains(&Value::String(
            ".orchid/packets/l_test-worker.md".to_string()
        )));
    assert!(payload["deleted"]
        .as_array()
        .unwrap()
        .contains(&Value::String(".orchid/reports/l_test.md".to_string())));
    assert!(!repo.root.join(".orchid").exists());

    let repo = Repo::new();
    let payload = repo.run(&["research-path", "specs/example", "--create"]);
    assert_eq!(payload["path"], ".orchid/spec-research/example");
    fs::write(
        repo.root
            .join(payload["path"].as_str().unwrap())
            .join("notes.md"),
        "temporary notes\n",
    )
    .unwrap();
    let payload = repo.run(&["research-clean", "example"]);
    assert_eq!(
        payload["deleted"],
        serde_json::json!([".orchid/spec-research/example"])
    );
    assert!(!repo.root.join(".orchid").exists());
}

#[test]
fn security_lock_and_help_contracts() {
    let repo = Repo::new();
    let payload = repo.run_fail(&["research-path", "../outside", "--create"]);
    assert_eq!(payload["code"], "invalid_spec_id");
    assert!(!repo.root.join(".orchid").exists());

    let outside = repo.root.parent().unwrap().join("outside.md");
    fs::write(
        &outside,
        "+++\nlease_id = \"l_test\"\nstatus = \"ready_for_validation\"\n+++\n",
    )
    .unwrap();
    let payload = repo.run_fail(&["report-check", outside.to_str().unwrap()]);
    assert_eq!(payload["code"], "path_outside_repo");

    let lock_dir = repo.root.join(".orchid/locks");
    fs::create_dir_all(&lock_dir).unwrap();
    fs::write(lock_dir.join("state.lock"), "held\n").unwrap();
    let payload = repo.run_fail(&["lease", "example", "T001", "--owner", "worker:agent_123"]);
    assert_eq!(payload["code"], "runtime_lock_busy");

    let help = repo.run_help(&["lease"]);
    assert!(help.contains("--serial"));
    assert!(help.contains("--allow-parallel"));
    let help = repo.run_help(&["next"]);
    assert!(help.contains("--older-than"));
    let help = repo.run_help(&["complete"]);
    assert!(help.contains("--clean-spec-research"));
}

#[test]
fn remaining_public_commands_keep_their_json_contracts() {
    let repo = Repo::new();
    let payload = repo.run(&["status", "--spec", "example"]);
    assert_eq!(payload["tasks"], 3);
    assert_eq!(payload["counts"]["todo"], 2);
    assert_eq!(payload["counts"]["done"], 1);
    assert_eq!(payload["selected_specs"], serde_json::json!(["example"]));

    let lease = repo.run(&[
        "lease",
        "example",
        "T001",
        "--owner",
        "worker:agent_123",
        "--lease-id",
        "l_test",
    ]);
    let running = repo.run(&["running"]);
    assert_eq!(running["leases"][0]["lease_id"], "l_test");

    let heartbeat = repo.run(&["heartbeat", "l_test"]);
    assert_eq!(heartbeat["action"], "heartbeat");
    assert_eq!(heartbeat["lease_id"], "l_test");

    let lease_path = repo.root.join(".orchid/leases/l_test.json");
    let mut lease_json: Value =
        serde_json::from_str(&fs::read_to_string(&lease_path).expect("lease json")).unwrap();
    let old = (Utc::now() - Duration::hours(2)).to_rfc3339_opts(SecondsFormat::Secs, false);
    lease_json["started_at"] = Value::String(old.clone());
    lease_json["heartbeat_at"] = Value::String(old);
    fs::write(&lease_path, serde_json::to_string(&lease_json).unwrap()).expect("rewrite lease");
    let stale = repo.run(&["stale", "--older-than", "30m"]);
    assert_eq!(stale["stale"][0]["lease_id"], "l_test");

    fs::write(
        repo.root.join(lease["report"].as_str().unwrap()),
        "+++\nlease_id = \"l_test\"\nstatus = \"ready_for_validation\"\ncommands_run = []\nresult = \"passed\"\n+++\n\n## Summary\n",
    )
    .unwrap();
    let report = repo.run(&["report-check", ".orchid/reports/l_test.md"]);
    assert_eq!(report["action"], "report-check");
    assert_eq!(report["next"], "validation");

    let release = repo.run(&["release", "l_test", "--reason", "paused"]);
    assert_eq!(release["status"], "released");
    let close = repo.run(&["close", "--lease", "l_test"]);
    assert_eq!(close["action"], "close");

    let git_status = repo.run(&["git-status"]);
    assert_eq!(git_status["git"], false);
    assert_eq!(git_status["active_leases"], serde_json::json!([]));

    let lint = repo.run(&["lint"]);
    assert_eq!(lint["ok"], true);
    assert_eq!(lint["tasks"], 3);

    let repo = Repo::new();
    let block = repo.run(&["block", "example", "T001", "--reason", "needs decision"]);
    assert_eq!(block["action"], "block");
    assert_eq!(block["status"], "blocked");
    let next = repo.run(&["next", "--spec", "example", "--explain"]);
    assert_eq!(next["phase"], "blocked");
}
