use serde_json::json;

use super::*;
use crate::testenv::{self, isolated};

const ACME: &str = "acme.freshservice.com";

/// Envelope written by Python `_argon_encrypt`'s format with small KDF params
/// (passphrase `pw`, domain `acme.freshservice.com`, salt 0..16, nonce 0..12).
const PY_ENVELOPE: &str = r#"{"version": 1, "backend": "argon", "kdf": "argon2id", "cipher": "aes-256-gcm", "params": {"time_cost": 1, "memory_cost": 8, "parallelism": 1}, "salt": "AAECAwQFBgcICQoLDA0ODw==", "nonce": "AAECAwQFBgcICQoL", "data": "Hwe03Ywf5M9Ui4WEsaIRfDqLN/vLX7QkVomiCf8XkycGWfBBELnCcApnsiZwYdWLsoOlndzUmgeQzKxAB0aawahMjmhPL5CCDEPqB3G+T0Jt9nUM9WCRN1k/V8bNog=="}"#;
const PY_PLAINTEXT: &str =
    r#"{"saved_at": 1.0, "domain": "acme.freshservice.com", "cookies": {"_x_m": "m"}}"#;

fn full() -> Cookies {
    [
        ("_x_m", "m"),
        ("_x_d", "d"),
        ("_x_w", "w"),
        ("fw-session-id", "f"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

fn session_json() -> Value {
    serde_json::from_str(&std::fs::read_to_string(config::session_file()).unwrap()).unwrap()
}

#[test]
fn store_pref_file_and_backend_selection() {
    let env = isolated().with_domain(ACME);
    assert_eq!(store_pref_file(), env.config_dir().join("store"));
    assert!(
        !keychain_available(),
        "FSV_NO_KEYCHAIN disables the keychain"
    );
    assert_eq!(
        current_backend(),
        Backend::File,
        "no store file, no session, no keychain"
    );
    set_backend(Backend::Argon).unwrap();
    assert_eq!(std::fs::read_to_string(store_pref_file()).unwrap(), "argon");
    assert_eq!(current_backend(), Backend::Argon);
    std::fs::write(store_pref_file(), " keychain \n").unwrap();
    assert_eq!(current_backend(), Backend::Keychain);
    std::fs::write(store_pref_file(), "bogus").unwrap();
    assert_eq!(current_backend(), Backend::File);
    assert_eq!(Backend::parse("argon"), Some(Backend::Argon));
    assert_eq!(Backend::Keychain.to_string(), "keychain");
}

#[cfg(target_os = "macos")]
#[test]
fn keychain_available_on_macos_unless_disabled() {
    let _env = isolated();
    testenv::unset("FSV_NO_KEYCHAIN");
    assert!(keychain_available());
    testenv::set("FSV_NO_KEYCHAIN", "1");
    assert!(!keychain_available());
}

struct Panicking;

impl KeychainStore for Panicking {
    fn write(&self, _: &str, _: &str) -> bool {
        panic!("security must not run")
    }
    fn read(&self, _: &str) -> Option<String> {
        panic!("security must not run")
    }
    fn delete(&self, _: &str) {
        panic!("security must not run")
    }
}

#[test]
fn keychain_disabled_never_runs_security() {
    let _env = isolated();
    testenv::set("FSV_NO_KEYCHAIN", "1");
    assert_eq!(
        keychain_write_to("x", &Panicking).unwrap_err().to_string(),
        "keychain backend is macOS only"
    );
    assert_eq!(
        keychain_read_from(&Panicking).unwrap_err().to_string(),
        "keychain backend is macOS only"
    );
    assert_eq!(
        keychain_write("x").unwrap_err().to_string(),
        "keychain backend is macOS only"
    );
    assert_eq!(
        keychain_read().unwrap_err().to_string(),
        "keychain backend is macOS only"
    );
    keychain_delete();
}

#[test]
fn security_cli_argv_shapes() {
    let mut calls: Vec<(Vec<String>, Option<String>)> = Vec::new();
    let mut run = |args: &[&str], input: Option<&str>| -> std::io::Result<(i32, String)> {
        calls.push((
            args.iter().map(|s| s.to_string()).collect(),
            input.map(str::to_string),
        ));
        Ok((0, "eyJhIjogMX0=\n".into()))
    };
    assert!(security_write("fsv-session-abc", "eyJhIjogMX0=", &mut run));
    assert_eq!(
        security_read("fsv-session-abc", &mut run).as_deref(),
        Some("eyJhIjogMX0=\n")
    );
    security_delete("fsv-session-abc", &mut run);
    let mut fail =
        |_: &[&str], _: Option<&str>| -> std::io::Result<(i32, String)> { Ok((44, String::new())) };
    assert!(!security_write("a", "b", &mut fail));
    assert_eq!(security_read("a", &mut fail), None);
    assert_eq!(
        calls[0],
        (
            vec!["-i".to_string()],
            Some(
                "add-generic-password -U -a fsv-session-abc -s fsv -w eyJhIjogMX0=\n".to_string()
            )
        )
    );
    assert_eq!(
        calls[1],
        (
            [
                "find-generic-password",
                "-a",
                "fsv-session-abc",
                "-s",
                "fsv",
                "-w"
            ]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
            None
        )
    );
    assert_eq!(
        calls[2].0,
        [
            "delete-generic-password",
            "-a",
            "fsv-session-abc",
            "-s",
            "fsv"
        ]
    );
}

#[test]
fn keychain_decode_base64_or_legacy_json() {
    for (raw, want) in [
        ("eyJhIjogMX0=", "{\"a\": 1}"),
        ("  {\"a\": 1}\n", "{\"a\": 1}"),
        ("not-base64!!", "not-base64!!"),
        ("w6k=\n", "é"),
        ("//79", "//79"),
    ] {
        assert_eq!(keychain_decode(raw), want, "{raw:?}");
    }
    assert_eq!(
        keychain_write_line("fsv-session", "eyJhIjogMX0="),
        "add-generic-password -U -a fsv-session -s fsv -w eyJhIjogMX0=\n"
    );
}

fn expected() -> Value {
    serde_json::from_str(include_str!("../../tests/fixtures/session_expected.json")).unwrap()
}

fn bytes(v: &Value) -> Vec<u8> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect()
}

/// `_b64e` / `_b64d` outputs and error texts from `gen_fsv_core_session.py`.
#[test]
fn b64_matches_python() {
    let x = expected();
    for row in x["b64e"].as_array().unwrap() {
        assert_eq!(b64e(&bytes(&row[0])), row[1].as_str().unwrap(), "{row}");
    }
    for row in x["b64d"].as_array().unwrap() {
        let input = row[0].as_str().unwrap();
        match &row[1] {
            Value::String(err) => {
                assert_eq!(
                    format!("Error: {}", b64d(input).unwrap_err()),
                    *err,
                    "{row}"
                );
            }
            want => assert_eq!(b64d(input).unwrap(), bytes(want), "{row}"),
        }
    }
}

#[test]
fn passphrase_matches_python() {
    for row in expected()["passphrase"].as_array().unwrap() {
        let mut answers = row["answers"].as_array().unwrap().iter();
        let mut prompts: Vec<String> = Vec::new();
        let got = passphrase(row["confirm"].as_bool().unwrap(), &mut |p: &str| {
            prompts.push(p.to_string());
            Ok(answers.next().unwrap().as_str().unwrap().to_string())
        });
        assert_eq!(json!(prompts), row["prompts"], "{row}");
        match &row["out"] {
            Value::String(want) => assert_eq!(&got.unwrap(), want, "{row}"),
            want => assert_eq!(got.unwrap_err().to_string(), want["error"], "{row}"),
        }
    }
    let aborted = passphrase(false, &mut |_: &str| Err(Error::Aborted));
    assert!(matches!(aborted, Err(Error::Aborted)));
}

#[test]
fn argon_encrypt_prompts_twice_and_binds_domain() {
    let _env = isolated().with_domain(ACME);
    let mut prompts = Vec::new();
    let raw = argon_encrypt("hello", &mut |p: &str| {
        prompts.push(p.to_string());
        Ok("pw".into())
    })
    .unwrap();
    assert_eq!(prompts, ["Passphrase: ", "Confirm passphrase: "]);
    assert_eq!(argon_decrypt_with(&raw, "pw", ACME).unwrap(), "hello");
    let mut answers = ["pw", "other"].into_iter();
    let err = argon_encrypt("hello", &mut |_: &str| Ok(answers.next().unwrap().into()));
    assert_eq!(err.unwrap_err().to_string(), "passphrases do not match");
}

#[test]
fn derive_key_matches_python_argon2() {
    let k = derive_key(
        "pw",
        b"0123456789abcdef",
        KdfParams {
            time_cost: 1,
            memory_cost: 8,
            parallelism: 1,
        },
    )
    .unwrap();
    assert_eq!(
        hex::encode(k),
        "2d27fc7564b6ac383fe503cb888b442eab1aa34a89b65f02d859e617a0dda66b"
    );
    let k = derive_key(
        "pässword",
        b"saltsaltsaltsalt",
        KdfParams {
            time_cost: 2,
            memory_cost: 16,
            parallelism: 2,
        },
    )
    .unwrap();
    assert_eq!(
        hex::encode(k),
        "ccf60b91124fd90f4e5aac2512365420eaf7a80a377b37a6a9655e5f1536d422"
    );
}

#[test]
fn argon_decrypts_python_envelope() {
    assert_eq!(
        argon_decrypt_with(PY_ENVELOPE, "pw", ACME).unwrap(),
        PY_PLAINTEXT
    );
    assert_eq!(
        argon_decrypt_with(PY_ENVELOPE, "wrong", ACME)
            .unwrap_err()
            .to_string(),
        "wrong passphrase or corrupted session"
    );
    assert_eq!(
        argon_decrypt_with(PY_ENVELOPE, "pw", "globex.freshservice.com")
            .unwrap_err()
            .to_string(),
        "wrong passphrase or corrupted session",
        "the domain is the AAD"
    );
    assert_eq!(
        argon_params(PY_ENVELOPE).unwrap(),
        KdfParams {
            time_cost: 1,
            memory_cost: 8,
            parallelism: 1
        }
    );
}

#[test]
fn argon_rejects_bad_envelopes() {
    let base: Value = serde_json::from_str(PY_ENVELOPE).unwrap();
    let with = |k: &str, v: Value| {
        let mut e = base.clone();
        e[k] = v;
        e.to_string()
    };
    let unreasonable = "invalid encrypted session file (unreasonable KDF parameters)";
    for params in [
        json!({"time_cost": 0, "memory_cost": 8, "parallelism": 1}),
        json!({"time_cost": 1, "memory_cost": 4, "parallelism": 1}),
        json!({"time_cost": 1, "memory_cost": 8, "parallelism": 17}),
        json!({"time_cost": "1", "memory_cost": 8, "parallelism": 1}),
        json!({}),
    ] {
        let raw = with("params", params.clone());
        assert_eq!(
            argon_params(&raw).unwrap_err().to_string(),
            unreasonable,
            "{params}"
        );
        // Checked before the passphrase prompt.
        assert_eq!(argon_decrypt(&raw).unwrap_err().to_string(), unreasonable);
    }
    assert_eq!(
        argon_params(&with("kdf", json!("scrypt")))
            .unwrap_err()
            .to_string(),
        "invalid encrypted session file"
    );
    assert_eq!(
        argon_decrypt(&with("kdf", json!("scrypt")))
            .unwrap_err()
            .to_string(),
        "invalid encrypted session file"
    );
}

#[test]
fn argon_encrypt_envelope_round_trips() {
    let raw = argon_encrypt_with("hello", "pw", ACME).unwrap();
    let doc: Value = serde_json::from_str(&raw).unwrap();
    let mut keys: Vec<&str> = doc
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "backend", "cipher", "data", "kdf", "nonce", "params", "salt", "version"
        ]
    );
    assert_eq!(
        doc["params"],
        json!({"time_cost": 3, "memory_cost": 65536, "parallelism": 4})
    );
    assert_eq!(
        (doc["version"].clone(), doc["cipher"].clone()),
        (json!(1), json!("aes-256-gcm"))
    );
    assert_eq!(B64.decode(doc["salt"].as_str().unwrap()).unwrap().len(), 16);
    assert_eq!(
        B64.decode(doc["nonce"].as_str().unwrap()).unwrap().len(),
        12
    );
    assert_eq!(argon_decrypt_with(&raw, "pw", ACME).unwrap(), "hello");
}

#[test]
fn file_session_save_load_update_logout() {
    let _env = isolated().with_domain(ACME);
    assert_eq!(
        read_file_session(Backend::File).unwrap_err().to_string(),
        "no session; run `fsv auth login`"
    );
    assert_eq!(
        load_cookies().unwrap_err().to_string(),
        "no session; run `fsv auth login`"
    );
    assert_eq!(session_age_hours(), None);

    assert_eq!(save_cookies(&full(), None).unwrap(), Backend::File);
    let doc = session_json();
    assert_eq!(
        doc.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["saved_at", "domain", "cookies"]
    );
    assert_eq!(doc["domain"], ACME);
    assert_eq!(
        doc["cookies"],
        json!({"_x_m": "m", "_x_d": "d", "_x_w": "w", "fw-session-id": "f"})
    );
    assert_eq!(std::fs::read_to_string(store_pref_file()).unwrap(), "file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(config::session_file())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert_eq!(current_backend(), Backend::File);
    assert_eq!(load_cookies().unwrap(), full());
    assert_eq!(
        cookie_header().unwrap(),
        "_x_m=m; _x_d=d; _x_w=w; fw-session-id=f"
    );
    let age = session_age_hours().unwrap();
    assert!((0.0..0.01).contains(&age), "{age}");

    update_cookie("fw-session-id", "f2");
    assert_eq!(session_json()["cookies"]["fw-session-id"], "f2");
    clear_cache();
    assert_eq!(load_cookies().unwrap()["fw-session-id"], "f2");
    update_cookie("new", "v");
    clear_cache();
    let mut want = full();
    want.insert("fw-session-id".into(), "f2".into());
    want.insert("new".into(), "v".into());
    assert_eq!(load_cookies().unwrap(), want);

    logout();
    assert!(!config::session_file().exists());
    assert!(!store_pref_file().exists());
    assert_eq!(current_backend(), Backend::File);
    assert!(
        load_cookies().is_err(),
        "the in-process cache is cleared too"
    );
}

#[test]
fn read_file_session_uses_cache_per_backend() {
    let _env = isolated().with_domain(ACME);
    ensure_dirs().unwrap();
    std::fs::write(config::session_file(), "one").unwrap();
    assert_eq!(read_file_session(Backend::File).unwrap(), "one");
    std::fs::write(config::session_file(), "two").unwrap();
    assert_eq!(read_file_session(Backend::File).unwrap(), "one", "cached");
    std::fs::write(
        config::session_file(),
        PY_ENVELOPE.replace("argon2id", "scrypt"),
    )
    .unwrap();
    assert_eq!(
        read_file_session(Backend::Argon).unwrap_err().to_string(),
        "invalid encrypted session file",
        "another backend bypasses the cache"
    );
}

#[test]
fn load_cookies_checks_domain_and_required_cookies() {
    let env = isolated().with_domain(ACME);
    save_cookies(&full(), None).unwrap();
    let _env = env.with_domain("globex.freshservice.com");
    clear_cache();
    assert_eq!(
        load_cookies().unwrap_err().to_string(),
        "session is for acme.freshservice.com, not globex.freshservice.com; run `fsv auth login`"
    );
    let _env = _env.with_domain(ACME);
    let partial: Cookies = [("_x_m".to_string(), "m".to_string())]
        .into_iter()
        .collect();
    save_cookies(&partial, Some(Backend::File)).unwrap();
    clear_cache();
    assert_eq!(
        load_cookies().unwrap_err().to_string(),
        "session missing cookies ['_x_d', '_x_w', 'fw-session-id']; run `fsv auth login`"
    );
}

#[test]
fn keychain_backend_without_keychain_fails() {
    let _env = isolated().with_domain(ACME);
    assert_eq!(
        save_cookies(&full(), Some(Backend::Keychain))
            .unwrap_err()
            .to_string(),
        "keychain backend is macOS only"
    );
    set_backend(Backend::Keychain).unwrap();
    assert_eq!(
        load_cookies().unwrap_err().to_string(),
        "keychain backend is macOS only"
    );
    update_cookie("x", "y");
    assert_eq!(session_age_hours(), None);
}

#[test]
fn update_cookie_is_silent_on_argon_and_errors() {
    let _env = isolated().with_domain(ACME);
    update_cookie("a", "b");
    assert!(
        !config::session_file().exists(),
        "no session: nothing written"
    );
    ensure_dirs().unwrap();
    std::fs::write(config::session_file(), PY_ENVELOPE).unwrap();
    set_backend(Backend::Argon).unwrap();
    update_cookie("a", "b");
    assert_eq!(
        std::fs::read_to_string(config::session_file()).unwrap(),
        PY_ENVELOPE
    );
}

#[test]
fn save_cookies_requires_domain() {
    let _env = isolated();
    assert!(
        save_cookies(&full(), Some(Backend::File))
            .unwrap_err()
            .to_string()
            .starts_with("no Freshservice domain")
    );
}

#[test]
fn parse_cookie_header_like_python() {
    let pairs = |s: &str| parse_cookie_header(s).into_iter().collect::<Vec<_>>();
    let kv = |k: &str, v: &str| (k.to_string(), v.to_string());
    assert_eq!(pairs("a=1; b=2"), [kv("a", "1"), kv("b", "2")]);
    assert_eq!(
        pairs(" _x_m = m ; bad; =x; y=; z=a=b "),
        [kv("_x_m", "m"), kv("z", "a=b")]
    );
    assert_eq!(pairs(""), []);
    assert_eq!(pairs("k=v;k=w"), [kv("k", "w")]);
}

#[test]
fn validate_lists_missing_cookies() {
    assert!(validate(&full()).is_ok());
    let one: Cookies = [("_x_m".to_string(), "1".to_string())]
        .into_iter()
        .collect();
    assert_eq!(
        validate(&one).unwrap_err().to_string(),
        "missing required cookies: ['_x_d', '_x_w', 'fw-session-id']"
    );
    assert_eq!(
        validate(&Cookies::new()).unwrap_err().to_string(),
        "missing required cookies: ['_x_m', '_x_d', '_x_w', 'fw-session-id']"
    );
}

#[test]
fn extract_cookie_from_every_paste_format() {
    for (raw, want) in [
        ("_x_m=a; _x_d=b", "_x_m=a; _x_d=b"),
        ("Host: x\nCookie: _x_m=a; b=c\nAccept: */*", "_x_m=a; b=c"),
        (
            ":authority: x\ncookie\n_x_m=a; b=2\naccept\n*/*",
            "_x_m=a; b=2",
        ),
        (
            "curl 'https://x' -H 'accept: */*' -H 'Cookie: _x_m=a; b=1' --compressed",
            "_x_m=a; b=1",
        ),
        ("curl \"u\" -H \"cookie: q=1\"", "q=1"),
        (
            r#"[{"name":"Accept","value":"x"},{"name":"Cookie","value":" _x_m=z "}]"#,
            "_x_m=z",
        ),
        (r#"{"headers":{"Cookie":"h=1"}}"#, "h=1"),
        (r#"{"cookie": "c=2"}"#, "c=2"),
        (r#"{"headers": "x"}"#, r#"{"headers": "x"}"#),
        ("  bare=1  \n", "bare=1"),
        ("cookie\n", "cookie"),
        ("COOKIE: up=1", "up=1"),
        (r#"[{"name":"cookie","value":5}]"#, "5"),
    ] {
        assert_eq!(extract_cookie_from_input(raw), want, "{raw:?}");
    }
}

#[test]
fn login_interactive_parses_and_validates_paste() {
    let _env = isolated().with_domain(ACME);
    let run = |raw: &'static str| {
        let mut prompts = Vec::new();
        let r = login_interactive_with(&mut |p: &str| {
            prompts.push(p.to_string());
            Ok(raw.to_string())
        });
        assert_eq!(prompts, ["> "]);
        r.map(|c| c.into_iter().collect::<Vec<_>>())
            .map_err(|e| e.to_string())
    };
    assert_eq!(run(""), Err("empty input".into()));
    assert_eq!(
        run("   "),
        Err("could not find Cookie header in pasted input".into())
    );
    assert_eq!(
        run("Cookie: _x_m=1; _x_d=2; _x_w=3; fw-session-id=4"),
        Ok(["_x_m", "_x_d", "_x_w", "fw-session-id"]
            .iter()
            .zip(["1", "2", "3", "4"])
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect())
    );
    assert_eq!(
        run("_x_m=1"),
        Err("missing required cookies: ['_x_d', '_x_w', 'fw-session-id']".into())
    );
    let aborted = login_interactive_with(&mut |_: &str| Err(Error::Aborted));
    assert!(matches!(aborted, Err(Error::Aborted)));
}

#[derive(Default)]
struct FakeKeychain {
    items: Mutex<std::collections::BTreeMap<String, String>>,
}

impl KeychainStore for FakeKeychain {
    fn write(&self, account: &str, encoded: &str) -> bool {
        self.items
            .lock()
            .unwrap()
            .insert(account.to_string(), encoded.to_string());
        true
    }
    fn read(&self, account: &str) -> Option<String> {
        self.items.lock().unwrap().get(account).cloned()
    }
    fn delete(&self, account: &str) {
        self.items.lock().unwrap().remove(account);
    }
}

impl FakeKeychain {
    fn accounts(&self) -> Vec<String> {
        self.items.lock().unwrap().keys().cloned().collect()
    }
}

struct Fake {
    keychain: std::sync::Arc<FakeKeychain>,
    home: Option<std::ffi::OsString>,
}

impl Drop for Fake {
    fn drop(&mut self) {
        set_keychain_store(None);
        match &self.home {
            Some(h) => testenv::set("HOME", h),
            None => testenv::unset("HOME"),
        }
    }
}

fn fake_keychain(env: &testenv::Isolated) -> Fake {
    let keychain = std::sync::Arc::new(FakeKeychain::default());
    set_keychain_store(Some(keychain.clone()));
    let home = std::env::var_os("HOME");
    testenv::set("HOME", env.root().join("home"));
    Fake { keychain, home }
}

fn use_dir(path: &std::path::Path) {
    std::fs::create_dir_all(path).unwrap();
    std::fs::write(path.join("config.json"), "{\"domain\": \"acme.freshservice.com\"}").unwrap();
    testenv::set("FSV_CONFIG_DIR", path);
    config::reload();
    clear_cache();
}

fn with_id(id: &str) -> Cookies {
    let mut c = full();
    c.insert("fw-session-id".into(), id.into());
    c
}

fn legacy_item(fake: &Fake) -> String {
    let legacy = serde_json::to_string(&json!({
        "saved_at": 1, "domain": ACME,
        "cookies": {"_x_m": "m", "_x_d": "d", "_x_w": "w", "fw-session-id": "f"}
    }))
    .unwrap();
    let enc = B64.encode(legacy.as_bytes());
    fake.keychain
        .items
        .lock()
        .unwrap()
        .insert(KEYCHAIN_ACCOUNT.into(), enc.clone());
    enc
}

#[test]
fn account_is_scoped_to_the_resolved_config_dir() {
    let env = isolated();
    let _fake = fake_keychain(&env);
    use_dir(&env.root().join("a"));
    let digest = sha256_hex_of(&crate::journal::resolve_path(&env.root().join("a")));
    assert_eq!(keychain_account(), format!("fsv-session-{}", &digest[..12]));
    let first = keychain_account();
    use_dir(&env.root().join("b"));
    assert_ne!(keychain_account(), first);
}

fn sha256_hex_of(p: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    crate::journal::sha256_hex(p.as_os_str().as_bytes())
}

#[test]
fn logout_of_one_config_dir_leaves_the_other() {
    let env = isolated();
    let fake = fake_keychain(&env);
    use_dir(&env.root().join("a"));
    save_cookies(&full(), Some(Backend::Keychain)).unwrap();
    let account_a = keychain_account();
    use_dir(&env.root().join("b"));
    save_cookies(&with_id("other"), Some(Backend::Keychain)).unwrap();
    assert_eq!(fake.keychain.accounts().len(), 2);
    logout();
    assert_eq!(fake.keychain.accounts(), vec![account_a]);
    use_dir(&env.root().join("b"));
    std::fs::write(config::config_dir().join("store"), "keychain").unwrap();
    assert!(load_cookies().is_err());
    use_dir(&env.root().join("a"));
    assert_eq!(load_cookies().unwrap()["fw-session-id"], "f");
}

#[test]
fn file_save_in_a_scratch_dir_keeps_other_keychain_items() {
    let env = isolated();
    let fake = fake_keychain(&env);
    use_dir(&env.root().join("real"));
    save_cookies(&full(), Some(Backend::Keychain)).unwrap();
    use_dir(&env.root().join("scratch"));
    save_cookies(&full(), Some(Backend::File)).unwrap();
    assert_eq!(fake.keychain.accounts().len(), 1);
    assert_eq!(current_backend(), Backend::File);
    logout();
    assert_eq!(fake.keychain.accounts().len(), 1);
    assert!(load_cookies().is_err());
}

#[test]
fn legacy_unscoped_item_only_for_the_default_config_dir() {
    let env = isolated();
    let fake = fake_keychain(&env);
    legacy_item(&fake);
    use_dir(&env.root().join("scratch"));
    std::fs::write(config::config_dir().join("store"), "keychain").unwrap();
    assert!(load_cookies().is_err());
    logout();
    assert!(fake.keychain.accounts().contains(&KEYCHAIN_ACCOUNT.to_string()));
    let default = env.root().join("home").join(".config").join("fsv");
    use_dir(&default);
    std::fs::write(default.join("store"), "keychain").unwrap();
    assert_eq!(load_cookies().unwrap(), full());
    save_cookies(&with_id("new"), Some(Backend::Keychain)).unwrap();
    assert_eq!(load_cookies().unwrap()["fw-session-id"], "new");
    logout();
    assert!(fake.keychain.accounts().is_empty());
}

#[test]
fn default_dir_without_env_override_falls_back_to_the_legacy_item() {
    let env = isolated();
    let fake = fake_keychain(&env);
    let legacy = legacy_item(&fake);
    use_dir(&env.root().join("scratch"));
    let scratch = keychain_account();
    save_cookies(&with_id("scratch"), Some(Backend::Keychain)).unwrap();
    let default = env.root().join("home").join(".config").join("fsv");
    std::fs::create_dir_all(&default).unwrap();
    std::fs::write(default.join("config.json"), "{\"domain\": \"acme.freshservice.com\"}").unwrap();
    std::fs::write(default.join("store"), "keychain").unwrap();
    testenv::unset("FSV_CONFIG_DIR");
    config::reload();
    clear_cache();
    assert!(!fake.keychain.accounts().contains(&keychain_account()));
    assert_eq!(load_cookies().unwrap(), full());
    logout();
    assert_eq!(fake.keychain.accounts(), vec![scratch]);
    assert_ne!(legacy, "");
}
