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
    assert_eq!(payload["ready"][0]["task"], "example/T001");

    let payload = repo.run(&["ready", "--spec", "example", "--explain"]);
    assert_eq!(payload["ready"][0]["task"], "example/T001");
    assert_eq!(payload["blocked"][0]["reason"], "unmet dependency:T001");
    assert_eq!(payload["blocked"][1]["reason"], "status:done");

    let payload = repo.run_fail(&["ready"]);
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
    assert!(repo.root.join(".orchid/leases/l_one.json").exists());
    assert!(!repo.root.join(".orch").exists());
    let running = repo.run_from_cwd(&["running"]);
    assert_eq!(running["leases"][0]["id"], "l_one");
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
    let running = repo.run(&["running"]);
    assert_eq!(running["leases"].as_array().unwrap().len(), 2);
}

#[test]
fn invalid_lease_ids_are_rejected_before_runtime_file_access() {
    let repo = Repo::new();
    let outside_lease = repo.root.parent().unwrap().join("outside-lease.json");
    let outside_bud = repo.root.parent().unwrap().join("outside-bud.md");

    let hyphenated = repo.run_fail(&[
        "lease",
        "example",
        "T001",
        "--owner",
        "worker:agent_123",
        "--lease-id",
        "l-unsafe",
    ]);
    assert_eq!(hyphenated["code"], "invalid_lease_id");

    let lease = repo.run_fail(&[
        "lease",
        "example",
        "T001",
        "--owner",
        "worker:agent_123",
        "--lease-id",
        "../../../outside-lease",
    ]);
    assert_eq!(lease["code"], "invalid_lease_id");
    assert!(!outside_lease.exists());

    let instructions = repo.root.join("bud-instructions.md");
    fs::write(&instructions, "Do bud work.\n").unwrap();
    let bud = repo.run_fail(&[
        "bud",
        "--title",
        "Unsafe bud id",
        "--scope",
        "src/feature/",
        "--instructions",
        instructions.to_str().unwrap(),
        "--lease-id",
        "../../../outside-bud",
    ]);
    assert_eq!(bud["code"], "invalid_lease_id");
    assert!(!outside_bud.exists());
    assert!(!repo.root.join(".orchid").exists());

    let missing_instruction = repo.run_fail(&[
        "bud",
        "--title",
        "Missing instructions loses to bad id",
        "--scope",
        "src/feature/",
        "--instructions",
        repo.root.join("missing.md").to_str().unwrap(),
        "--lease-id",
        "../../../outside-bud",
    ]);
    assert_eq!(missing_instruction["code"], "invalid_lease_id");

    let lock_dir = repo.root.join(".orchid/locks");
    fs::create_dir_all(&lock_dir).unwrap();
    fs::write(lock_dir.join("state.lock"), "{}\n").unwrap();
    let close = repo.run_fail(&["close", "--lease", "../../../outside-lease", "--force"]);
    assert_eq!(close["code"], "invalid_lease_id");
    assert!(lock_dir.join("state.lock").exists());
    fs::remove_file(lock_dir.join("state.lock")).unwrap();

    let lease_dir = repo.root.join(".orchid/leases");
    fs::create_dir_all(&lease_dir).unwrap();
    let task_one = repo.root.join("specs/example/tasks/T001.md");
    let task_two = repo.root.join("specs/example/tasks/T002.md");
    let task_one_before = fs::read_to_string(&task_one).unwrap();
    let task_two_before = fs::read_to_string(&task_two).unwrap();
    fs::write(
        lease_dir.join("l_evil.json"),
        serde_json::json!({
            "lease_id": "l_evil",
            "status": "completed",
            "report_path": "specs/example/tasks/T001.md",
            "instructions_path": "specs/example/tasks/T002.md"
        })
        .to_string(),
    )
    .unwrap();
    let cleanup = repo.run(&["cleanup", "--completed"]);
    assert_eq!(cleanup["closed"], serde_json::json!(["l_evil"]));
    assert_eq!(fs::read_to_string(&task_one).unwrap(), task_one_before);
    assert_eq!(fs::read_to_string(&task_two).unwrap(), task_two_before);
    assert!(!lease_dir.join("l_evil.json").exists());

    fs::create_dir_all(&lease_dir).unwrap();
    fs::write(
        lease_dir.join("l_victim.json"),
        serde_json::json!({
            "lease_id": "l_victim",
            "status": "completed"
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        lease_dir.join("aaa.json"),
        serde_json::json!({
            "lease_id": "l_victim",
            "status": "completed"
        })
        .to_string(),
    )
    .unwrap();
    let cleanup = repo.run_fail(&["cleanup", "--completed"]);
    assert_eq!(cleanup["code"], "invalid_lease_id");
    assert!(lease_dir.join("l_victim.json").exists());
    fs::remove_file(lease_dir.join("aaa.json")).unwrap();
    fs::remove_file(lease_dir.join("l_victim.json")).unwrap();

    fs::create_dir_all(&lease_dir).unwrap();
    fs::write(
        lease_dir.join("malicious.json"),
        serde_json::json!({
            "lease_id": "../../../outside-lease",
            "status": "completed"
        })
        .to_string(),
    )
    .unwrap();
    let cleanup = repo.run_fail(&["cleanup", "--completed"]);
    assert_eq!(cleanup["code"], "invalid_lease_id");
    assert!(!outside_lease.exists());
}

#[test]
#[cfg(unix)]
fn symlinked_repo_task_paths_are_rejected_before_outside_write() {
    let repo = Repo::new();
    let outside_spec = repo.root.parent().unwrap().join("outside/evil");
    let outside_task_dir = outside_spec.join("tasks");
    fs::create_dir_all(&outside_task_dir).unwrap();
    let outside_task = outside_task_dir.join("T001.md");
    let task_text = "+++\nid = \"T001\"\ntitle = \"outside\"\nstatus = \"todo\"\nscope = [\"src/evil/\"]\ndepends = []\ncovers = []\nverification_mode = \"mayor\"\nverification_status = \"pending\"\n+++\n\n## Context\n";
    fs::write(&outside_task, task_text).unwrap();
    std::os::unix::fs::symlink(&outside_spec, repo.root.join("specs/evil")).unwrap();

    let blocked = repo.run_fail(&["block", "evil", "T001", "--reason", "outside-write"]);
    assert_eq!(blocked["code"], "path_outside_repo");
    assert_eq!(fs::read_to_string(outside_task).unwrap(), task_text);
}

#[test]
#[cfg(unix)]
fn atomic_write_rejects_preexisting_temp_symlink() {
    let repo = Repo::new();
    let outside_tmp_target = repo.root.parent().unwrap().join("outside-temp-target");
    fs::write(&outside_tmp_target, "keep me\n").unwrap();
    let tmp_name = format!(".l_test.json.{}.0.tmp", std::process::id());
    fs::create_dir_all(repo.root.join(".orchid/leases")).unwrap();
    std::os::unix::fs::symlink(
        &outside_tmp_target,
        repo.root.join(".orchid/leases").join(tmp_name),
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
    assert_eq!(fs::read_to_string(outside_tmp_target).unwrap(), "keep me\n");
}

#[test]
#[cfg(unix)]
fn symlinked_runtime_dirs_are_rejected_before_outside_delete() {
    let repo = Repo::new();
    let orch = repo.root.join(".orchid");
    let leases = orch.join("leases");
    fs::create_dir_all(&leases).unwrap();
    fs::write(
        leases.join("l_link.json"),
        serde_json::json!({
            "lease_id": "l_link",
            "status": "completed"
        })
        .to_string(),
    )
    .unwrap();
    let outside_reports = repo.root.parent().unwrap().join("outside-reports");
    fs::create_dir_all(&outside_reports).unwrap();
    let outside_report = outside_reports.join("l_link.md");
    fs::write(&outside_report, "keep me\n").unwrap();
    std::os::unix::fs::symlink(&outside_reports, orch.join("reports")).unwrap();

    let cleanup = repo.run_fail(&["cleanup", "--completed"]);
    assert_eq!(cleanup["code"], "path_outside_repo");
    assert_eq!(fs::read_to_string(outside_report).unwrap(), "keep me\n");
}

#[test]
#[cfg(unix)]
fn symlinked_orchid_root_is_rejected_before_lock_creation() {
    let repo = Repo::new();
    let outside_orchid = repo.root.parent().unwrap().join("outside-orchid");
    fs::create_dir_all(&outside_orchid).unwrap();
    std::os::unix::fs::symlink(&outside_orchid, repo.root.join(".orchid")).unwrap();

    let cleanup = repo.run_fail(&["cleanup", "--completed"]);
    assert_eq!(cleanup["code"], "path_outside_repo");
    assert!(!outside_orchid.join("locks/state.lock").exists());
}

#[test]
#[cfg(unix)]
fn symlinked_lease_dir_is_rejected_for_direct_lease_reads() {
    let repo = Repo::new();
    let orch = repo.root.join(".orchid");
    fs::create_dir_all(&orch).unwrap();
    let outside_leases = repo.root.parent().unwrap().join("outside-leases");
    fs::create_dir_all(&outside_leases).unwrap();
    fs::write(
        outside_leases.join("l_link.json"),
        serde_json::json!({
            "lease_id": "l_link",
            "status": "active",
            "task": "example/T001"
        })
        .to_string(),
    )
    .unwrap();
    std::os::unix::fs::symlink(&outside_leases, orch.join("leases")).unwrap();

    let heartbeat = repo.run_fail(&["heartbeat", "l_link"]);
    assert_eq!(heartbeat["code"], "path_outside_repo");
}

#[test]
#[cfg(unix)]
fn symlinked_lease_files_are_rejected_for_aggregate_reads() {
    let repo = Repo::new();
    let leases = repo.root.join(".orchid/leases");
    fs::create_dir_all(&leases).unwrap();
    let outside_lease = repo.root.parent().unwrap().join("outside-l_link.json");
    fs::write(
        &outside_lease,
        serde_json::json!({
            "lease_id": "l_link",
            "status": "active",
            "task": "example/T001"
        })
        .to_string(),
    )
    .unwrap();
    std::os::unix::fs::symlink(&outside_lease, leases.join("l_link.json")).unwrap();

    let running = repo.run_fail(&["running"]);
    assert_eq!(running["code"], "path_outside_repo");
}

#[test]
#[cfg(unix)]
fn symlinked_spec_directories_are_rejected_before_enumeration() {
    let repo = Repo::new();
    let outside_spec = repo.root.parent().unwrap().join("outside-enum");
    fs::create_dir_all(outside_spec.join("tasks")).unwrap();
    std::os::unix::fs::symlink(&outside_spec, repo.root.join("specs/evil-enum")).unwrap();

    let ready = repo.run_fail(&["ready", "--all-open"]);
    assert_eq!(ready["code"], "path_outside_repo");
}

#[test]
#[cfg(unix)]
fn symlinked_spec_research_root_is_rejected_before_create() {
    let repo = Repo::new();
    let orch = repo.root.join(".orchid");
    fs::create_dir_all(&orch).unwrap();
    let outside_research = repo.root.parent().unwrap().join("outside-research");
    fs::create_dir_all(&outside_research).unwrap();
    std::os::unix::fs::symlink(&outside_research, orch.join("spec-research")).unwrap();

    let path = repo.run_fail(&["research-path", "example", "--create"]);
    assert_eq!(path["code"], "path_outside_repo");
    assert!(!outside_research.join("example").exists());
}

#[test]
#[cfg(unix)]
fn symlinked_spec_sidecars_are_rejected_before_packet_read() {
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
    let outside_requirements = repo.root.parent().unwrap().join("outside-requirements.md");
    fs::write(&outside_requirements, "outside requirements\n").unwrap();
    let requirements = repo.root.join("specs/example/requirements.md");
    fs::remove_file(&requirements).unwrap();
    std::os::unix::fs::symlink(&outside_requirements, requirements).unwrap();

    let packet = repo.run_fail(&["packet", "--lease", "l_test", "--role", "worker"]);
    assert_eq!(packet["code"], "path_outside_repo");
}

#[test]
fn lease_agent_metadata_attach_and_status_lookup_work() {
    let repo = Repo::new();
    repo.run(&[
        "lease",
        "example",
        "T001",
        "--owner",
        "worker:agent_123",
        "--agent-id",
        "agent_123",
        "--lease-id",
        "l_agent",
    ]);
    let status = repo.run(&["status", "--agent-id", "agent_123"]);
    assert_eq!(status["agent_id"], "agent_123");
    assert_eq!(status["lease_id"], "l_agent");
    assert_eq!(status["kind"], "task");
    assert_eq!(status["status"], "active");
    assert_eq!(status["task"], "example/T001");
    assert_eq!(status["report"], ".orchid/reports/l_agent.md");

    repo.write_task_file("example", "T005", "todo", "src/other/");
    repo.run(&[
        "lease",
        "example",
        "T005",
        "--owner",
        "worker:unassigned",
        "--lease-id",
        "l_attach",
        "--allow-parallel",
    ]);
    let attach = repo.run(&[
        "lease-attach-agent",
        "--lease",
        "l_attach",
        "--agent-id",
        "agent_456",
    ]);
    assert_eq!(attach["lease_id"], "l_attach");
    assert_eq!(attach["agent_id"], "agent_456");
    let status = repo.run(&["status", "--agent-id", "agent_456"]);
    assert_eq!(status["lease_id"], "l_attach");
    assert_eq!(status["owner"], "worker:agent_456");

    let missing = repo.run_fail(&["status", "--agent-id", "agent_missing"]);
    assert_eq!(missing["code"], "agent_lease_not_found");
    let combined = repo.run_fail(&["status", "--agent-id", "agent_123", "--spec", "example"]);
    assert_eq!(combined["code"], "scope_selector_conflict");
    let duplicate = repo.run_fail(&[
        "lease-attach-agent",
        "--lease",
        "l_attach",
        "--agent-id",
        "agent_123",
    ]);
    assert_eq!(duplicate["code"], "agent_id_already_attached");
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
fn bud_creates_runtime_packet_without_report_stub() {
    let repo = Repo::new();
    let instructions = repo.root.join("bud-instructions.md");
    fs::write(&instructions, "Diagnose the runner failure.\n").unwrap();
    let payload = repo.run(&[
        "bud",
        "--title",
        "Diagnose runner failure",
        "--scope",
        "src/feature/",
        "--instructions",
        instructions.to_str().unwrap(),
        "--agent-id",
        "agent_123",
        "--lease-id",
        "l_bud",
    ]);
    assert_eq!(payload["lease_id"], "l_bud");
    assert_eq!(payload["kind"], "bud");
    assert_eq!(payload["task"], "bud:l_bud");
    assert_eq!(payload["owner"], "worker:agent_123");
    assert_eq!(payload["agent_id"], "agent_123");
    assert_eq!(payload["packet"], ".orchid/packets/l_bud-worker.md");
    assert_eq!(payload["report"], ".orchid/reports/l_bud.md");
    assert!(repo.root.join(".orchid/leases/l_bud.json").exists());
    assert!(repo.root.join(".orchid/buds/l_bud.md").exists());
    assert!(!repo.root.join(".orchid/reports/l_bud.md").exists());

    let packet = fs::read_to_string(repo.root.join(".orchid/packets/l_bud-worker.md")).unwrap();
    assert!(packet.contains("## Bud Instructions"));
    assert!(packet.contains("Diagnose the runner failure."));
    assert!(packet.contains("Do not call Orchid lifecycle commands."));

    let status = repo.run(&["status", "--agent-id", "agent_123"]);
    assert_eq!(status["lease_id"], "l_bud");
    assert_eq!(status["kind"], "bud");
    assert_eq!(status["packet"], ".orchid/packets/l_bud-worker.md");

    let validator_packet = repo.run(&["packet", "--lease", "l_bud", "--role", "validator"]);
    assert_eq!(
        validator_packet["packet"],
        ".orchid/packets/l_bud-validator.md"
    );
    let status = repo.run(&["status", "--agent-id", "agent_123"]);
    assert_eq!(status["packet"], ".orchid/packets/l_bud-worker.md");
}

#[test]
fn bud_enforces_scope_and_parallel_guards() {
    let repo = Repo::new();
    let instructions = repo.root.join("bud-instructions.md");
    fs::write(&instructions, "Work.\n").unwrap();
    let missing_scope = repo.run_fail(&[
        "bud",
        "--title",
        "Missing scope",
        "--instructions",
        instructions.to_str().unwrap(),
    ]);
    assert_eq!(missing_scope["code"], "scope_required");

    repo.run(&[
        "bud",
        "--title",
        "First",
        "--scope",
        "src/feature/",
        "--instructions",
        instructions.to_str().unwrap(),
        "--lease-id",
        "l_bud_one",
    ]);
    let overlap = repo.run_fail(&[
        "bud",
        "--title",
        "Overlap",
        "--scope",
        "src/feature/file.rs",
        "--instructions",
        instructions.to_str().unwrap(),
        "--lease-id",
        "l_bud_two",
        "--allow-parallel",
    ]);
    assert_eq!(overlap["code"], "scope_conflict");
    let serial = repo.run_fail(&[
        "bud",
        "--title",
        "Serial",
        "--scope",
        "src/other/",
        "--instructions",
        instructions.to_str().unwrap(),
        "--lease-id",
        "l_bud_three",
        "--serial",
    ]);
    assert_eq!(serial["code"], "serial_blocked");
    let parallel_required = repo.run_fail(&[
        "bud",
        "--title",
        "Parallel required",
        "--scope",
        "src/other/",
        "--instructions",
        instructions.to_str().unwrap(),
        "--lease-id",
        "l_bud_four",
    ]);
    assert_eq!(parallel_required["code"], "parallel_not_confirmed");

    let duplicate_id = repo.run_fail(&[
        "bud",
        "--title",
        "Duplicate id",
        "--scope",
        "src/other/",
        "--instructions",
        instructions.to_str().unwrap(),
        "--lease-id",
        "l_bud_one",
        "--allow-parallel",
    ]);
    assert_eq!(duplicate_id["code"], "lease_id_already_exists");
}

#[test]
fn next_moves_through_dispatch_wait_validate_and_recover() {
    let repo = Repo::new();
    let payload = repo.run(&["next", "--spec", "example", "--explain"]);
    assert_eq!(payload["phase"], "dispatch");
    assert_eq!(
        payload["cmd"],
        serde_json::json!(["lease", "example", "T001", "--owner", "worker:<agent-id>"])
    );

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
    assert_eq!(payload["stale"][0]["id"], "l_test");
}

#[test]
fn bud_packet_complete_git_and_cleanup_lifecycle_work() {
    let repo = Repo::new();
    repo.init_git();
    let instructions = repo.root.join("bud-instructions.md");
    fs::write(&instructions, "Change feature work only.\n").unwrap();
    let payload = repo.run(&[
        "bud",
        "--title",
        "Feature bud",
        "--scope",
        "src/feature/",
        "--instructions",
        instructions.to_str().unwrap(),
        "--lease-id",
        "l_bud",
    ]);
    fs::write(
        repo.root.join("src/feature/work.txt"),
        "changed during bud\n",
    )
    .unwrap();
    let touched = repo.run(&["git-touched", "--lease", "l_bud"]);
    assert_eq!(
        touched["stage"],
        serde_json::json!(["src/feature/work.txt"])
    );
    let stage = repo.run(&["git-stage-plan", "--lease", "l_bud"]);
    assert_eq!(
        stage["pathspecs"],
        serde_json::json!(["src/feature/work.txt"])
    );

    fs::write(
        repo.root.join(payload["report"].as_str().unwrap()),
        "+++\nlease_id = \"l_bud\"\nstatus = \"ready_for_validation\"\ncommands_run = []\nresult = \"passed\"\n+++\n\n## Summary\n",
    )
    .unwrap();
    let report = repo.run(&["report-check", ".orchid/reports/l_bud.md"]);
    assert_eq!(report["lease_id"], "l_bud");
    assert_eq!(report["task"], "bud:l_bud");
    let complete = repo.run(&["complete", "--lease", "l_bud", "--verified-by", "mayor"]);
    assert_eq!(complete["kind"], "bud");
    let lease_json: Value = serde_json::from_str(
        &fs::read_to_string(repo.root.join(".orchid/leases/l_bud.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(lease_json["status"], "completed");
    assert_eq!(lease_json["verified_by"], "mayor");

    let close = repo.run(&["close", "--lease", "l_bud"]);
    assert!(close["deleted"]
        .as_array()
        .unwrap()
        .contains(&Value::String(".orchid/buds/l_bud.md".to_string())));

    let repo = Repo::new();
    let instructions = repo.root.join("bud-instructions.md");
    fs::write(&instructions, "Cleanup bud.\n").unwrap();
    repo.run(&[
        "bud",
        "--title",
        "Cleanup bud",
        "--scope",
        "src/feature/",
        "--instructions",
        instructions.to_str().unwrap(),
        "--lease-id",
        "l_cleanup",
    ]);
    repo.run(&["complete", "--lease", "l_cleanup", "--verified-by", "mayor"]);
    let cleanup = repo.run(&["cleanup", "--completed"]);
    assert!(cleanup["deleted"]
        .as_array()
        .unwrap()
        .contains(&Value::String(".orchid/buds/l_cleanup.md".to_string())));
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
    assert_eq!(payload["lease_id"], "l_test");
    assert_eq!(payload["task"], "example/T001");
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
fn report_check_rejects_report_path_that_claims_another_lease() {
    let repo = Repo::new();
    repo.write_task_file("example", "T005", "todo", "src/other/");
    repo.run(&[
        "lease",
        "example",
        "T001",
        "--owner",
        "worker:agent_123",
        "--lease-id",
        "l_a",
    ]);
    repo.run(&[
        "lease",
        "example",
        "T005",
        "--owner",
        "worker:agent_456",
        "--lease-id",
        "l_b",
        "--allow-parallel",
    ]);
    fs::write(
        repo.root.join(".orchid/reports/l_a.md"),
        "+++\nlease_id = \"l_b\"\nstatus = \"ready_for_validation\"\ncommands_run = []\nresult = \"passed\"\n+++\n\n## Summary\n",
    )
    .unwrap();

    let next = repo.run(&["next", "--spec", "example", "--explain"]);
    assert_eq!(
        next["cmds"][0],
        serde_json::json!(["report-check", ".orchid/reports/l_a.md"])
    );

    let report = repo.run_fail(&["report-check", ".orchid/reports/l_a.md"]);
    assert_eq!(report["code"], "report_lease_mismatch");
    assert_eq!(report["lease_id"], "l_b");
    assert_eq!(report["report"], ".orchid/reports/l_a.md");
    assert_eq!(report["expected_report"], ".orchid/reports/l_b.md");
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
fn complete_preserves_inline_task_arrays() {
    let repo = Repo::new();
    let task_path = repo.root.join("specs/example/tasks/T001.md");

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
    assert!(rewritten.contains("scope = [\"src/feature/\"]"));
    assert!(rewritten.contains("covers = [\"R001\"]"));
    assert!(!rewritten.contains("scope = [\n    \"src/feature/\",\n]"));
    assert!(!rewritten.contains("covers = [\n    \"R001\",\n]"));
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
        payload["stage"],
        serde_json::json!(["src/feature/work.txt"])
    );
    assert_eq!(
        payload["blocked_by"]["out_of_scope"],
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

    let help = repo.run_help(&[]);
    assert!(help.contains("List ready task files"));
    assert!(help.contains("Generate a worker, validator, reviewer, or loop-runner packet"));

    let help = repo.run_help(&["lease"]);
    assert!(help.contains("Task target: SPEC with TASK_ID"));
    assert!(help.contains("Lease owner label"));
    assert!(help.contains("--agent-id"));
    assert!(help.contains("--serial"));
    assert!(help.contains("--allow-parallel"));
    let help = repo.run_help(&["bud"]);
    assert!(help.contains("Short title for the bud delegation"));
    assert!(help.contains("--instructions"));
    assert!(help.contains("--scope"));
    let help = repo.run_help(&["lease-attach-agent"]);
    assert!(help.contains("Discovery-only runtime agent id to attach"));
    let help = repo.run_help(&["status"]);
    assert!(help.contains("Find the lease attached to a discovery-only agent id"));
    let help = repo.run_help(&["next"]);
    assert!(help.contains("Include recommended action, queues, and blockers"));
    assert!(help.contains("--older-than"));
    let help = repo.run_help(&["complete"]);
    assert!(help.contains("Independent review reference for the commit"));
    assert!(help.contains("--clean-spec-research"));
}

#[test]
fn remaining_public_commands_keep_their_json_contracts() {
    let repo = Repo::new();
    let payload = repo.run(&["status", "--spec", "example"]);
    assert_eq!(payload["tasks"], 3);
    assert_eq!(payload["counts"]["todo"], 2);
    assert_eq!(payload["counts"]["done"], 1);

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
    assert_eq!(running["leases"][0]["id"], "l_test");

    let heartbeat = repo.run(&["heartbeat", "l_test"]);
    assert_eq!(heartbeat["lease_id"], "l_test");

    let lease_path = repo.root.join(".orchid/leases/l_test.json");
    let mut lease_json: Value =
        serde_json::from_str(&fs::read_to_string(&lease_path).expect("lease json")).unwrap();
    let old = (Utc::now() - Duration::hours(2)).to_rfc3339_opts(SecondsFormat::Secs, false);
    lease_json["started_at"] = Value::String(old.clone());
    lease_json["heartbeat_at"] = Value::String(old);
    fs::write(&lease_path, serde_json::to_string(&lease_json).unwrap()).expect("rewrite lease");
    let stale = repo.run(&["stale", "--older-than", "30m"]);
    assert_eq!(stale["stale"][0]["id"], "l_test");

    fs::write(
        repo.root.join(lease["report"].as_str().unwrap()),
        "+++\nlease_id = \"l_test\"\nstatus = \"ready_for_validation\"\ncommands_run = []\nresult = \"passed\"\n+++\n\n## Summary\n",
    )
    .unwrap();
    let report = repo.run(&["report-check", ".orchid/reports/l_test.md"]);
    assert_eq!(report["next"], "validation");

    let release = repo.run(&["release", "l_test", "--reason", "paused"]);
    assert_eq!(release["lease_id"], "l_test");
    let close = repo.run(&["close", "--lease", "l_test"]);
    assert_eq!(close["lease_id"], "l_test");

    let git_status = repo.run(&["git-status"]);
    assert_eq!(git_status["git"], false);

    let lint = repo.run(&["lint"]);
    assert_eq!(lint["tasks"], 3);

    let repo = Repo::new();
    let block = repo.run(&["block", "example", "T001", "--reason", "needs decision"]);
    assert_eq!(block["task"], "example/T001");
    let next = repo.run(&["next", "--spec", "example", "--explain"]);
    assert_eq!(next["phase"], "blocked");
}
