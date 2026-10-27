use hhh_im_protocol::{
    Outgoing, Sticker, StickerCaps, StickerPage, method::Method, platform::Capabilities,
    valid_sticker_id,
};
use serde_json::json;

#[test]
fn old_plugins_stay_closed_and_sticker_only_plugins_are_writable() {
    let old: Capabilities = serde_json::from_value(json!({})).unwrap();
    assert_eq!(old.stickers, StickerCaps::NONE);
    assert!(old.is_read_only());
    let caps: Capabilities =
        serde_json::from_value(json!({"stickers":{"send":true,"favorites":true}})).unwrap();
    assert!(!caps.is_read_only());
    assert!(!caps.stickers.favorite_add);
    assert!(!caps.stickers.packs);
}

#[test]
fn opaque_ids_optional_favorite_and_paging_round_trip() {
    let data =
        json!({"items":[{"id":"abc:/贴纸","name":"收到","imageKey":"key:v2"}],"next":"page:2"});
    let page: StickerPage<Sticker> = serde_json::from_value(data.clone()).unwrap();
    assert_eq!(page.items[0].favorite, None);
    assert_eq!(serde_json::to_value(page).unwrap(), data);
    assert!(valid_sticker_id("abc:/贴纸"));
    for invalid in ["", "bad\n", &"x".repeat(513)] {
        assert!(!valid_sticker_id(invalid));
    }
    let outgoing: Outgoing =
        serde_json::from_value(json!({"type":"sticker","id":"abc:/贴纸"})).unwrap();
    assert!(matches!(outgoing, Outgoing::Sticker { .. }));
    for method in [
        Method::StickersPacks,
        Method::StickersList,
        Method::StickersFavorites,
        Method::StickersFavorite,
        Method::StickersImage,
    ] {
        assert_eq!(Method::from_name(method.as_str()), Some(method));
    }
}
