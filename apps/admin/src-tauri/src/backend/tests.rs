use super::*;
#[test]
fn public_catalog_and_status_hide_secrets() {
    let t = tempfile::tempdir().unwrap();
    let b = Backend::new(t.path().join("config.json"), t.path().to_path_buf());
    b.save(json!({"projectPath":"private/project","buildNodes":[{"id":"one","name":"公开名字","profileAssetPath":"Assets/private.asset","target":13,"keystorePass":"SECRET"}],"buildGroups":[{"id":"g","name":"组","nodeIds":["one","missing"]}]})).unwrap();
    let c = b.handle("GET", "/api/user/catalog", json!({})).unwrap();
    let s = c.to_string();
    assert!(!s.contains("SECRET"));
    assert!(!s.contains("Assets/private"));
    assert_eq!(c["groups"][0]["nodeIds"], json!(["one"]));
    *b.job.lock().unwrap() = json!({"state":"done","queue":[{"name":"公开名字","targetName":"Android","output":"secret/path"}],"end":{"ok":false,"reason":"error","message":"SECRET"}});
    let s = b.public_status().to_string();
    assert!(!s.contains("SECRET"));
    assert!(!s.contains("secret/path"));
    assert_eq!(
        b.handle(
            "POST",
            "/api/user/build",
            json!({"nodeIds":["one"],"engine":{}})
        )
        .unwrap_err()
        .0,
        400
    );
    assert_eq!(
        b.handle("POST", "/api/user/build", json!({"nodeIds":["missing"]}))
            .unwrap_err()
            .0,
        400
    );
}
#[test]
fn task_acquisition_is_exclusive() {
    let t = tempfile::tempdir().unwrap();
    let b = Backend::new(t.path().join("c"), t.path().into());
    b.acquire().unwrap();
    assert_eq!(b.acquire().unwrap_err().0, 409);
    b.stop();
    assert!(b.cancelled());
    b.release();
    b.acquire().unwrap();
    assert!(!b.cancelled());
    b.release();
}
#[test]
fn process_failure_and_timeout_are_real() {
    let t = tempfile::tempdir().unwrap();
    let b = Backend::new(t.path().join("c"), t.path().into());
    b.acquire().unwrap();
    #[cfg(windows)]
    let (exe, args) = (
        "cmd",
        vec![
            "/d".into(),
            "/c".into(),
            "echo test-output & exit /b 7".into(),
        ],
    );
    #[cfg(not(windows))]
    let (exe, args) = ("sh", vec!["-c".into(), "echo test-output; exit 7".into()]);
    let log = t.path().join("run.log");
    assert_eq!(
        b.run_process(
            exe,
            &args,
            t.path(),
            &log,
            "line",
            0,
            Duration::from_secs(5)
        )
        .unwrap(),
        7
    );
    assert!(fs::read_to_string(log).unwrap().contains("test-output"));
    #[cfg(windows)]
    let (exe, args) = (
        "cmd",
        vec!["/d".into(), "/c".into(), "ping -n 10 127.0.0.1 >nul".into()],
    );
    #[cfg(not(windows))]
    let (exe, args) = ("sh", vec!["-c".into(), "sleep 10".into()]);
    let start = Instant::now();
    assert!(b
        .run_process(
            exe,
            &args,
            t.path(),
            &t.path().join("timeout.log"),
            "line",
            0,
            Duration::from_millis(100)
        )
        .is_err());
    assert!(start.elapsed() < Duration::from_secs(5));
    b.release();
}
