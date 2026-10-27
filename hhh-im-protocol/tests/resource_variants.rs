use hhh_im_protocol::method::ResourcesReadParams;
use hhh_im_protocol::{ResourceVariant, ResourceVariants};
use serde_json::json;

#[test]
fn variants_preserve_originals_and_select_a_fitting_preview() {
    let variants: ResourceVariants = serde_json::from_value(json!({"variants": [
        {"id":"png-360","preview":true,"mime":"image/png","width":270,"height":360},
        {"id":"png-1024","preview":true,"mime":"image/png","width":768,"height":1024},
        {"id":"original","original":true,"mime":"image/heic","name":"photo.HEIC","bytes":2256293}
    ]}))
    .unwrap();
    assert!(variants.validate().is_ok());
    assert_eq!(variants.preview(320).unwrap().id, "png-360");
    assert_eq!(variants.preview(800).unwrap().id, "png-1024");
    assert_eq!(variants.preview(2048).unwrap().id, "png-1024");
    assert_eq!(
        variants.original().unwrap().name.as_deref(),
        Some("photo.HEIC")
    );
    let single = ResourceVariants {
        variants: vec![ResourceVariant {
            id: "same".into(),
            preview: true,
            original: true,
            ..Default::default()
        }],
    };
    assert_eq!(
        single.preview(512).unwrap().id,
        single.original().unwrap().id
    );
}

#[test]
fn legacy_reads_omit_the_variant_and_variant_streams_have_different_keys() {
    let old = json!({"conversationId":"chat","messageId":"message","key":"file","kind":"image","offset":0,"length":32});
    let params: ResourcesReadParams = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(&params).unwrap(), old);
    let mut preview = params.clone();
    preview.variant = Some("png-1024".into());
    let mut original = preview.clone();
    original.variant = Some("original".into());
    assert_ne!(preview.segment_key(), original.segment_key());
}

#[test]
fn invalid_or_ambiguous_variants_are_refused() {
    for value in [
        json!({"variants":[{"id":"none"}]}),
        json!({"variants":[{"id":"x","preview":true},{"id":"x","original":true}]}),
        json!({"variants":[{"id":"a","original":true},{"id":"b","original":true}]}),
        json!({"variants":[{"id":"a","preview":true,"width":0,"height":5}]}),
        json!({"variants":[{"id":"a\nsecret","preview":true}]}),
    ] {
        let variants: ResourceVariants = serde_json::from_value(value).unwrap();
        assert!(variants.validate().is_err());
    }
}

#[test]
fn describing_versions_requires_resource_reads_and_is_optional_for_old_plugins() {
    use hhh_im_protocol::platform::{Capabilities, HistoryMode};
    let mut caps = Capabilities::default();
    caps.messages.list = true;
    assert!(caps.check_consistent(HistoryMode::Server).is_ok());
    assert!(
        !serde_json::to_value(&caps).unwrap()["resources"]
            .as_object()
            .unwrap()
            .contains_key("variants")
    );
    caps.resources.variants = true;
    assert!(caps.check_consistent(HistoryMode::Server).is_err());
    caps.resources.read = true;
    assert!(caps.check_consistent(HistoryMode::Server).is_ok());
}
