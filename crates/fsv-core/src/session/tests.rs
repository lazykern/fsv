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

#[test]
fn keychain_disabled_never_runs_security() {
    let _env = isolated();
    let mut ran = false;
    let mut run = |_: &[&str], _: Option<&str>| -> std::io::Result<(i32, String)> {
        ran = true;
        Ok((0, String::new()))
    };
    assert_eq!(
        keychain_write_with("x", &mut run).unwrap_err().to_string(),
        "keychain backend is macOS only"
    );
    assert_eq!(
        keychain_read_with(&mut run).unwrap_err().to_string(),
        "keychain backend is macOS only"
    );
    assert!(!ran);
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

#[cfg(target_os = "macos")]
#[test]
fn keychain_write_and_read_through_security_cli() {
    let _env = isolated();
    testenv::unset("FSV_NO_KEYCHAIN");
    let mut calls: Vec<(Vec<String>, Option<String>)> = Vec::new();
    let mut run = |args: &[&str], input: Option<&str>| -> std::io::Result<(i32, String)> {
        calls.push((
            args.iter().map(|s| s.to_string()).collect(),
            input.map(str::to_string),
        ));
        Ok((0, "eyJhIjogMX0=\n".into()))
    };
    keychain_write_with("{\"a\": 1}", &mut run).unwrap();
    assert_eq!(keychain_read_with(&mut run).unwrap(), "{\"a\": 1}");
    let mut fail =
        |_: &[&str], _: Option<&str>| -> std::io::Result<(i32, String)> { Ok((44, String::new())) };
    let write_err = keychain_write_with("x", &mut fail).unwrap_err().to_string();
    let read_err = keychain_read_with(&mut fail).unwrap_err().to_string();
    testenv::set("FSV_NO_KEYCHAIN", "1");
    assert_eq!(
        calls[0],
        (
            vec!["-i".to_string()],
            Some("add-generic-password -U -a fsv-session -s fsv -w eyJhIjogMX0=\n".to_string())
        )
    );
    assert_eq!(
        calls[1].0,
        [
            "find-generic-password",
            "-a",
            "fsv-session",
            "-s",
            "fsv",
            "-w"
        ]
    );
    assert_eq!(calls[1].1, None);
    assert_eq!(write_err, "could not write session to Keychain");
    assert_eq!(read_err, "no session in Keychain; run `fsv auth login`");
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
        keychain_write_line("{\"a\": 1}"),
        "add-generic-password -U -a fsv-session -s fsv -w eyJhIjogMX0=\n"
    );
}

#[test]
fn b64_matches_python() {
    // `_b64e` / `_b64d` are the standard base64 engine.
    assert_eq!(B64.encode(b"\x00\xffhi"), "AP9oaQ==");
    assert_eq!(B64.decode("AP9oaQ==").unwrap(), b"\x00\xffhi");
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
