//! The built-in macros we write must be byte-identical to the files
//! Audacity 3.7.3 writes (~/.local/share/audacity/Macros); skipped if absent.
use au_script::macros::MacroCommands;

#[test]
fn builtin_macro_files_match_audacity() {
    let aud = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".local/share/audacity/Macros");
    if !aud.join("Fade Ends.txt").exists() {
        eprintln!("no Audacity macros; skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("macros-cmp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut mc = MacroCommands::new(&dir);
    for name in ["Fade Ends", "MP3 Conversion"] {
        assert!(mc.restore_macro(name));
        mc.write_macro(name).unwrap();
        let ours = std::fs::read(dir.join(format!("{name}.txt"))).unwrap();
        let theirs = std::fs::read(aud.join(format!("{name}.txt"))).unwrap();
        assert_eq!(String::from_utf8_lossy(&ours), String::from_utf8_lossy(&theirs), "{name}");
        // and reading Audacity's file back yields the same command list
        let mut mc2 = MacroCommands::new(&aud);
        mc2.read_macro(name).unwrap();
        assert_eq!(mc2.commands(), mc.commands());
    }
}
