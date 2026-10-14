//! Parses Audacity 3.7.3's real locale/*.po and audacity.cfg when available (AU_LOCALE_DIR / ~/.config/audacity).
use au_core::{i18n, prefs};
static LANG: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[test]
fn real_po_and_cfg() {
    let _g = LANG.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::var("AU_TEST_PO_DIR").unwrap_or("/workspace/loc".into());
    let de = std::path::Path::new(&dir).join("de.po");
    if let Ok(t) = std::fs::read_to_string(&de) {
        let c = i18n::Catalog::parse_po(&t).unwrap();
        assert!(c.len() > 1000, "{}", c.len());
        assert_eq!(c.get("&File"), Some("&Datei"));
        i18n::set_language("de", &[std::path::PathBuf::from(&dir)]).unwrap();
        assert_eq!(i18n::tr("&Edit"), "&Bearbeiten");
        i18n::set_language("en", &[]).unwrap();
    }
    let pl = std::path::Path::new(&dir).join("pl.po");
    if let Ok(t) = std::fs::read_to_string(&pl) {
        let c = i18n::Catalog::parse_po(&t).unwrap();
        assert!(c.len() > 1000);
    }
    let cfg = dirs_home().join(".config/audacity/audacity.cfg");
    if let Ok(t) = std::fs::read_to_string(&cfg) {
        let p = prefs::Prefs::parse_cfg(&t);
        let again = prefs::Prefs::parse_cfg(&p.to_cfg());
        assert_eq!(p.to_cfg(), again.to_cfg());
    }
}
fn dirs_home() -> std::path::PathBuf { std::env::var("HOME").map(Into::into).unwrap_or_default() }
#[test]
fn ui_labels_without_accelerators() {
    let _g = LANG.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::var("AU_TEST_PO_DIR").unwrap_or("/workspace/loc".into());
    if !std::path::Path::new(&dir).join("de.po").exists() { return; }
    i18n::set_language("de", &[std::path::PathBuf::from(&dir)]).unwrap();
    assert_eq!(i18n::tr_ui("File"), "Datei");
    assert_eq!(i18n::tr_ui("Edit"), "Bearbeiten");
    assert_eq!(i18n::tr_ui("No such label 123"), "No such label 123");
    i18n::set_language("en", &[]).unwrap();
    assert_eq!(i18n::tr_ui("File"), "File");
}
