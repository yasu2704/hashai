use std::collections::BTreeMap;

use hashai::{
    ExitCode,
    config::{Config, ConfigOverrides, ConfigSources, Keybinding},
};

#[test]
fn ac1_precedence_resolves_each_editor_field_independently() {
    let user = Config {
        trigger: ",,".into(),
        trigger_enabled: false,
        keybinding: Keybinding::CtrlX,
        ..Config::default()
    };
    let environment = BTreeMap::from([
        ("HASHAI_TRIGGER".into(), "env ".into()),
        ("HASHAI_TRIGGER_ENABLED".into(), "true".into()),
        ("HASHAI_KEYBINDING".into(), "ctrl-g".into()),
    ]);
    let resolved = ConfigSources::resolve(
        Some(user),
        &environment,
        ConfigOverrides {
            trigger: Some("cli ".into()),
            trigger_enabled: Some(false),
            keybinding: Some("ctrl-x".into()),
            ..ConfigOverrides::default()
        },
    )
    .unwrap();
    assert_eq!(resolved.trigger, "cli ");
    assert!(!resolved.trigger_enabled);
    assert_eq!(resolved.keybinding, Keybinding::CtrlX);
}

#[test]
fn ac1_each_editor_field_has_an_independent_precedence_ladder() {
    let user = Config {
        trigger: "user".into(),
        trigger_enabled: false,
        keybinding: Keybinding::CtrlX,
        ..Config::default()
    };
    let defaults =
        ConfigSources::resolve(None, &BTreeMap::new(), ConfigOverrides::default()).unwrap();
    assert_eq!(defaults.trigger, "# ");
    assert!(defaults.trigger_enabled);
    assert_eq!(defaults.keybinding, Keybinding::CtrlG);
    let user_only = ConfigSources::resolve(
        Some(user.clone()),
        &BTreeMap::new(),
        ConfigOverrides::default(),
    )
    .unwrap();
    assert_eq!(user_only.trigger, "user");
    assert!(!user_only.trigger_enabled);
    assert_eq!(user_only.keybinding, Keybinding::CtrlX);
    let env = BTreeMap::from([
        ("HASHAI_TRIGGER".into(), "env".into()),
        ("HASHAI_TRIGGER_ENABLED".into(), "true".into()),
        ("HASHAI_KEYBINDING".into(), "ctrl-g".into()),
    ]);
    let environment = ConfigSources::resolve(Some(user), &env, ConfigOverrides::default()).unwrap();
    assert_eq!(environment.trigger, "env");
    assert!(environment.trigger_enabled);
    assert_eq!(environment.keybinding, Keybinding::CtrlG);
    let cli = ConfigSources::resolve(
        None,
        &BTreeMap::new(),
        ConfigOverrides {
            trigger: Some("cli".into()),
            ..ConfigOverrides::default()
        },
    )
    .unwrap();
    assert_eq!(cli.trigger, "cli");
    assert!(cli.trigger_enabled);
    assert_eq!(cli.keybinding, Keybinding::CtrlG);
}

#[test]
fn ac3_disabled_is_a_boolean_not_a_magic_trigger() {
    let literal = ConfigSources::resolve(
        None,
        &BTreeMap::from([("HASHAI_TRIGGER".into(), "disabled".into())]),
        ConfigOverrides::default(),
    )
    .unwrap();
    assert_eq!(literal.trigger, "disabled");
    assert!(literal.trigger_enabled);
    let disabled = ConfigSources::resolve(
        None,
        &BTreeMap::from([("HASHAI_TRIGGER_ENABLED".into(), "false".into())]),
        ConfigOverrides::default(),
    )
    .unwrap();
    assert!(!disabled.trigger_enabled);
}

#[test]
fn ac5_editor_validation_rejects_bad_inputs_with_argument_errors() {
    for trigger in [
        "".to_owned(),
        "a\n".to_owned(),
        "a\r".to_owned(),
        "a\0".to_owned(),
        "x".repeat(65),
    ] {
        let error = ConfigSources::resolve(
            None,
            &BTreeMap::from([("HASHAI_TRIGGER".into(), trigger)]),
            ConfigOverrides::default(),
        )
        .unwrap_err();
        assert_eq!(error.exit_code(), ExitCode::ArgumentOrConfig as i32);
    }
    for (name, value) in [
        ("HASHAI_TRIGGER_ENABLED", "TRUE"),
        ("HASHAI_KEYBINDING", "alt-g"),
    ] {
        let error = ConfigSources::resolve(
            None,
            &BTreeMap::from([(name.into(), value.into())]),
            ConfigOverrides::default(),
        )
        .unwrap_err();
        assert_eq!(error.exit_code(), ExitCode::ArgumentOrConfig as i32);
    }
}
