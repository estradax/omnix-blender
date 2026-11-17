#![allow(dead_code)]
//! Shared test helpers: a real headless Blender harness, an MCP client for the built binary, and a fake add-on.

use omnix_blender::link::Link;
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    service::RunningService,
    transport::TokioChildProcess,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

pub type Client = RunningService<RoleClient, ()>;

/// Resolve Blender or return None (tests then print a SKIP line and pass).
pub fn blender_bin() -> Option<PathBuf> {
    omnix_blender::blender::find_blender()
}

#[macro_export]
macro_rules! require_blender {
    () => {
        match $crate::support::blender_bin() {
            Some(b) => b,
            None => {
                eprintln!("SKIP: Blender not found (install it or set OMNIX_BLENDER_BIN); skipping integration test");
                return;
            }
        }
    };
}

/// Short private temp dir under /tmp (AF_UNIX paths are limited to 103 bytes; mkdtemp creates it 0700).
pub fn short_tempdir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::Builder::new()
        .prefix("obt")
        .tempdir_in("/tmp")
        .expect("tempdir");
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    d
}

pub fn addon_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("addon")
}

/// A real Blender 5.3 running `addon/headless_runner.py` in an isolated environment.
pub struct Harness {
    pub dir: tempfile::TempDir,
    pub sock: PathBuf,
    pub bin: PathBuf,
    pub allow_run_python: bool,
    child: Option<Child>,
}

impl Harness {
    pub fn start(bin: &Path, allow_run_python: bool) -> Self {
        let mut h = Self::unstarted(bin, allow_run_python);
        h.spawn();
        h
    }

    pub fn unstarted(bin: &Path, allow_run_python: bool) -> Self {
        let dir = short_tempdir();
        let sock = dir.path().join("b.sock");
        Harness {
            dir,
            sock,
            bin: bin.to_path_buf(),
            allow_run_python,
            child: None,
        }
    }

    pub fn pid(&self) -> u32 {
        self.child.as_ref().expect("running").id()
    }

    pub fn blender_command(&self, bin: &Path) -> Command {
        let mut c = Command::new(bin);
        c.env("BLENDER_USER_RESOURCES", self.dir.path().join("res"))
            .env("BLENDER_USER_SCRIPTS", self.dir.path().join("scripts"))
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env_remove("OMNIX_BLENDER_SOCKET")
            .env_remove("OMNIX_BLENDER_ALLOW_RUN_PYTHON");
        c
    }

    pub fn spawn(&mut self) {
        if let Err(e) = self.try_spawn() {
            panic!("{e}");
        }
    }

    /// Start Blender and wait for the add-on to report ready; Err(log) if Blender exits first.
    pub fn try_spawn(&mut self) -> Result<(), String> {
        let _ = std::fs::remove_file(self.dir.path().join("out.txt"));
        let out = std::fs::File::create(self.dir.path().join("out.txt")).unwrap();
        let err = out.try_clone().unwrap();
        let mut c = self.blender_command(&self.bin.clone());
        c.args(["--background", "--factory-startup", "--python"])
            .arg(addon_dir().join("headless_runner.py"))
            .arg("--")
            .arg("--socket")
            .arg(&self.sock);
        if self.allow_run_python {
            c.arg("--allow-run-python");
        }
        let child = c.stdout(out).stderr(err).spawn().expect("spawn blender");
        self.child = Some(child);
        let start = Instant::now();
        loop {
            let log = std::fs::read_to_string(self.dir.path().join("out.txt")).unwrap_or_default();
            if log.contains("OMNIX_READY") {
                break;
            }
            if let Some(st) = self.child.as_mut().unwrap().try_wait().unwrap() {
                self.child = None;
                return Err(format!("blender exited early ({st}):\n{log}"));
            }
            assert!(
                start.elapsed() < Duration::from_secs(120),
                "blender did not become ready:\n{log}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }

    /// Ask Blender to quit cleanly via the headless-only method, then reap it.
    pub async fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            let link = Link::new(&self.sock);
            let _ = link
                .call(
                    "headless.shutdown",
                    json!({}),
                    Duration::from_secs(5),
                    false,
                )
                .await;
            let start = Instant::now();
            while child.try_wait().unwrap().is_none() {
                if start.elapsed() > Duration::from_secs(15) {
                    let _ = child.kill();
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = child.wait();
        }
    }

    /// Kill Blender abruptly (simulates a crash: the socket file is left behind).
    pub fn crash(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("out.txt")).unwrap_or_default()
    }

    /// An MCP client talking to the built `omnix-blender serve` binary over stdio.
    pub async fn client(&self) -> Client {
        connect_client(&self.sock).await
    }

    /// Independent channel straight to the add-on, used to inspect real Blender state with `run_python`.
    /// Only valid on a harness started with `allow_run_python = true`.
    pub async fn inspect(&self, code: &str) -> Value {
        assert!(self.allow_run_python, "inspect needs run_python enabled");
        let link = Link::new(&self.sock);
        let r = link
            .call(
                "run_python",
                json!({"code": code, "undo": false}),
                Duration::from_secs(30),
                false,
            )
            .await
            .expect("inspect run_python");
        assert!(
            r.get("error").is_none() || r["error"].is_null(),
            "inspect failed: {r}"
        );
        r["result"].clone()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

pub async fn connect_client(sock: &Path) -> Client {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_omnix-blender"));
    cmd.arg("serve").env("OMNIX_BLENDER_SOCKET", sock);
    ().serve(TokioChildProcess::new(cmd).expect("spawn omnix-blender"))
        .await
        .expect("MCP handshake")
}

pub struct Out {
    pub is_error: bool,
    pub text: String,
    pub raw: CallToolResult,
}

impl Out {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.text))
    }
}

pub async fn call(c: &Client, name: &str, args: Value) -> Out {
    let mut p = CallToolRequestParams::new(name.to_string());
    if let Value::Object(m) = args {
        p = p.with_arguments(m);
    }
    let raw = tokio::time::timeout(Duration::from_secs(120), c.call_tool(p))
        .await
        .expect("tool call hung")
        .unwrap_or_else(|e| panic!("protocol-level error calling {name}: {e}"));
    let text = raw
        .content
        .iter()
        .filter_map(|b| b.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    Out {
        is_error: raw.is_error == Some(true),
        text,
        raw,
    }
}

/// Call a tool and require success.
pub async fn ok(c: &Client, name: &str, args: Value) -> Value {
    let o = call(c, name, args).await;
    assert!(!o.is_error, "{name} failed: {}", o.text);
    o.json()
}

/// Call a tool and require an error whose text contains `code`.
pub async fn err(c: &Client, name: &str, args: Value, code: &str) -> String {
    let o = call(c, name, args).await;
    assert!(o.is_error, "{name} unexpectedly succeeded: {}", o.text);
    assert!(o.text.contains(code), "expected '{code}' in: {}", o.text);
    o.text
}

pub mod fake {
    //! A tiny fake add-on speaking the protocol, so link/MCP behaviour can be tested without Blender.
    use serde_json::{Value, json};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    pub enum Reply {
        Ok(Value),
        Err(&'static str, &'static str),
        /// Never answer (simulates a hung Blender).
        Silent,
        /// Write these raw bytes instead of a JSON response.
        Raw(Vec<u8>),
        /// Close the connection without answering.
        Close,
    }

    pub type Handler = Arc<dyn Fn(&str, &Value) -> Reply + Send + Sync>;

    pub struct Fake {
        pub path: PathBuf,
        pub seen: Arc<Mutex<Vec<(String, Value)>>>,
        task: tokio::task::JoinHandle<()>,
        conns: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
        pub hello_v: Arc<Mutex<u64>>,
    }

    impl Fake {
        pub fn start(path: &Path, handler: Handler) -> Self {
            let _ = std::fs::remove_file(path);
            let listener = UnixListener::bind(path).unwrap();
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
            let seen: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
            let hello_v = Arc::new(Mutex::new(1u64));
            let (s2, h2) = (seen.clone(), hello_v.clone());
            let conns: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> = Arc::default();
            let c2 = conns.clone();
            let task = tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    let (handler, seen, hv) = (handler.clone(), s2.clone(), h2.clone());
                    let jh = tokio::spawn(async move {
                        let (r, mut w) = stream.into_split();
                        let mut lines = BufReader::new(r).lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            let req: Value = serde_json::from_str(&line).unwrap();
                            let id = req["id"].clone();
                            let method = req["method"].as_str().unwrap().to_string();
                            if method == "hello" {
                                let v = *hv.lock().unwrap();
                                let resp = json!({"v":1,"id":id,"ok":true,"result":{"v":v,"blender":"5.3.0","addon":"0.1.0","features":["scene_info"],"headless":true}});
                                let _ = w.write_all(format!("{resp}\n").as_bytes()).await;
                                continue;
                            }
                            seen.lock()
                                .unwrap()
                                .push((method.clone(), req["params"].clone()));
                            match handler(&method, &req) {
                                Reply::Ok(v) => {
                                    let resp = json!({"v":1,"id":id,"ok":true,"result":v});
                                    let _ = w.write_all(format!("{resp}\n").as_bytes()).await;
                                }
                                Reply::Err(code, msg) => {
                                    let resp = json!({"v":1,"id":id,"ok":false,"error":{"code":code,"message":msg}});
                                    let _ = w.write_all(format!("{resp}\n").as_bytes()).await;
                                }
                                Reply::Silent => {}
                                Reply::Raw(b) => {
                                    let _ = w.write_all(&b).await;
                                }
                                Reply::Close => return,
                            }
                        }
                    });
                    c2.lock().unwrap().push(jh);
                }
            });
            Fake {
                path: path.to_path_buf(),
                seen,
                task,
                conns,
                hello_v,
            }
        }

        pub fn calls(&self, method: &str) -> usize {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == method)
                .count()
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self.task.abort();
            for c in self.conns.lock().unwrap().iter() {
                c.abort();
            }
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// An isolated Blender resource environment for tests that use the *installed* extension.
pub struct ExtEnv {
    pub dir: tempfile::TempDir,
    pub sock: PathBuf,
    pub bin: PathBuf,
}

impl ExtEnv {
    pub fn new(bin: &Path) -> Self {
        let dir = short_tempdir();
        let sock = dir.path().join("e.sock");
        ExtEnv {
            dir,
            sock,
            bin: bin.to_path_buf(),
        }
    }

    pub fn apply(&self, c: &mut Command) {
        c.env("BLENDER_USER_RESOURCES", self.dir.path().join("res"))
            .env("BLENDER_USER_SCRIPTS", self.dir.path().join("scripts"))
            .env("OMNIX_BLENDER_BIN", &self.bin)
            .env("OMNIX_BLENDER_SOCKET", &self.sock);
    }

    /// Run the built `omnix-blender` binary with this environment.
    pub fn omnix(&self, args: &[&str]) -> std::process::Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_omnix-blender"));
        c.args(args);
        self.apply(&mut c);
        c.output().expect("run omnix-blender")
    }

    /// Start Blender running `script` (no --factory-startup, so the installed extension can load).
    pub fn spawn_script(&self, script: &str, background: bool, extra: &[(&str, &str)]) -> Session {
        let log = self
            .dir
            .path()
            .join(format!("{}.log", script.replace('.', "_")));
        let out = std::fs::File::create(&log).unwrap();
        let err = out.try_clone().unwrap();
        let stop = self.dir.path().join("stop.flag");
        let _ = std::fs::remove_file(&stop);
        let mut c = Command::new(&self.bin);
        self.apply(&mut c);
        c.env("OMNIX_TEST_STOP", &stop);
        for (k, v) in extra {
            c.env(k, v);
        }
        if background {
            c.arg("--background");
        }
        c.arg("--python").arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/support")
                .join(script),
        );
        let child = c.stdout(out).stderr(err).spawn().expect("spawn blender");
        Session {
            child: Some(child),
            log,
            stop,
        }
    }
}

pub struct Session {
    child: Option<Child>,
    pub log: PathBuf,
    pub stop: PathBuf,
}

impl Session {
    pub fn text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Wait until the log contains `marker`; panics with the log on exit/timeout/failure marker.
    pub fn wait_for(&mut self, marker: &str, fail_marker: &str, secs: u64) {
        let start = Instant::now();
        loop {
            let t = self.text();
            assert!(!t.contains(fail_marker), "session failed:\n{t}");
            if t.contains(marker) {
                return;
            }
            if let Some(st) = self.child.as_mut().unwrap().try_wait().unwrap() {
                panic!("blender exited ({st}) before '{marker}':\n{t}");
            }
            assert!(
                start.elapsed() < Duration::from_secs(secs),
                "timeout waiting for '{marker}':\n{t}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn wait_exit(&mut self, secs: u64) -> std::process::ExitStatus {
        let mut child = self.child.take().unwrap();
        let start = Instant::now();
        loop {
            if let Some(st) = child.try_wait().unwrap() {
                return st;
            }
            if start.elapsed() > Duration::from_secs(secs) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("blender did not exit:\n{}", self.text());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Ask the session to finish (flag file) and wait for Blender to exit.
    pub fn finish(&mut self) -> std::process::ExitStatus {
        std::fs::write(&self.stop, b"stop").unwrap();
        self.wait_exit(60)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
