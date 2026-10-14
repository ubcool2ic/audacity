# Stage 10 split — UI parity and polish

## ubcool2ic agent (core, effects, IO, app logic)
- Preferences model: every Audacity 3.7.3 preference page's settings with the same keys/defaults (`audacity.cfg` import), applied in core/IO/effects (devices, quality/dither, import/export, recording, tracks behaviours, warnings).
- Keyboard shortcut compatibility: Audacity default + "Full" shortcut sets, import/export of Audacity's keyboard XML, command ids shared with the scripting vocabulary.
- Remaining menu commands and workflows in app logic (Edit/Select/View/Transport/Tracks/Generate/Effect/Analyze/Tools/Extra menus): track align/sort/mix-and-render/resample, clip boundaries, labelled-audio edits, Timer Record, Punch and Roll, Sound Activated Recording, Dependencies/Missing files, project rate/format menus.
- Localization infrastructure: gettext `.po` catalogs from Audacity's `locale/` (msgids as in Audacity), runtime language switch.
- audio.com cloud save/share: client logic against a mocked service (OAuth flow, upload/sync of `.aup3`), opt-in e2e test.

## BizyBot (runtime, plugin host, GUI hardening, packaging)
- Toolbars (Tools, Edit, Cut/Copy/Paste, Meter, Mixer, Transcription, Spectral, Time, Selection, Snapping, Audio Setup, Share), docking/layout persistence; themes (Light/Dark/High Contrast/Classic) incl. image cache format.
- Accessibility with AccessKit (accessible names/roles for track panel, keyboard navigation, screen-reader semantics) and its tests.
- Localization layout tests (no clipping/overlap in translated dialogs).
- UI automation suite under Xvfb (editing, recording, import/export, preferences, recovery workflows).
- Installers and CI: AppImage/.deb (Linux), .msi (Windows), signed .dmg (macOS), smoke tests in CI.
