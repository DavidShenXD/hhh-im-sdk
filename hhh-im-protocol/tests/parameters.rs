use hhh_im_protocol::platform::SettingDecl;

#[test]
fn parameter_fixture_keeps_old_fields_and_round_trips_new_optionals() {
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/parameters.json")).unwrap();
    let fields: Vec<SettingDecl> = serde_json::from_value(expected.clone()).unwrap();
    assert_eq!(fields[0].field_type, None);
    assert_eq!(fields[2].show_when.as_ref().unwrap().field, "retry");
    assert_eq!(serde_json::to_value(fields).unwrap(), expected);
    let old: SettingDecl = serde_json::from_str(r#"{"id":"endpoint","label":"地址"}"#).unwrap();
    assert_eq!(old.field_type, None);
    assert!(!old.secret);
    assert!(!old.required);
}
