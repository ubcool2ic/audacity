# Extra-menu commands split (2026-10-15)

These are the Extra-menu commands that are not yet implemented, in Audacity 3.7.3's menu order. The list comes from Audacity's own `GetInfo: Type=Menus` with ShowExtraMenus=1, then probing which ids our pipe answers with "not recognized" (probe at `8098b0e`).

The probe found **89** commands, not the 85 from my earlier report. That number came from an older probe that skipped Drag and CompareAudio and miscounted. The split is still into halves:
- **ubcool2ic: items 1–45.**
- **BizyBot: items 46–89.**

Each item has to become a real pipe/menu command with Audacity's default shortcut, verified against Audacity 3.7.3 over the pipe where possible.

| # | Id | Label | Submenu | Owner |
|---|---|---|---|---|
| 1 | PlayOneSec | Play One Second | Transport | ubcool2ic |
| 2 | PlayToSelection | Play to Selection | Transport | ubcool2ic |
| 3 | PlayBeforeSelectionStart | Play Before Selection Start | Transport | ubcool2ic |
| 4 | PlayAfterSelectionStart | Play After Selection Start | Transport | ubcool2ic |
| 5 | PlayBeforeSelectionEnd | Play Before Selection End | Transport | ubcool2ic |
| 6 | PlayAfterSelectionEnd | Play After Selection End | Transport | ubcool2ic |
| 7 | PlayBeforeAndAfterSelectionStart | Play Before and After Selection Start | Transport | ubcool2ic |
| 8 | PlayBeforeAndAfterSelectionEnd | Play Before and After Selection End | Transport | ubcool2ic |
| 9 | PlayCutPreview | Play Cut Preview | Transport | ubcool2ic |
| 10 | KeyboardScrubBackwards | Scrub Backwards | Transport | ubcool2ic |
| 11 | KeyboardScrubForwards | Scrub Forwards | Transport | ubcool2ic |
| 12 | SelectTool | Selection Tool | Tools | ubcool2ic |
| 13 | EnvelopeTool | Envelope Tool | Tools | ubcool2ic |
| 14 | DrawTool | Draw Tool | Tools | ubcool2ic |
| 15 | MultiTool | Multi Tool | Tools | ubcool2ic |
| 16 | PrevTool | Previous Tool | Tools | ubcool2ic |
| 17 | NextTool | Next Tool | Tools | ubcool2ic |
| 18 | OutputGain | Adjust Playback Volume... | Mixer | ubcool2ic |
| 19 | OutputGainInc | Increase Playback Volume | Mixer | ubcool2ic |
| 20 | OutputGainDec | Decrease Playback Volume | Mixer | ubcool2ic |
| 21 | InputGain | Adjust Recording Volume... | Mixer | ubcool2ic |
| 22 | InputGainInc | Increase Recording Volume | Mixer | ubcool2ic |
| 23 | InputGainDec | Decrease Recording Volume | Mixer | ubcool2ic |
| 24 | DeleteKey | Delete Key | Edit | ubcool2ic |
| 25 | DeleteKey2 | Delete Key2 | Edit | ubcool2ic |
| 26 | ClipLeft | Time Shift Left | Edit | ubcool2ic |
| 27 | ClipRight | Time Shift Right | Edit | ubcool2ic |
| 28 | PlayAtSpeed | Play-at-Speed Once | Play-at-Speed | ubcool2ic |
| 29 | PlayAtSpeedCutPreview | Play Cut Preview-at-Speed | Play-at-Speed | ubcool2ic |
| 30 | PlayAtSpeedLooped | Play-at-Speed | Play-at-Speed | ubcool2ic |
| 31 | PlaySpeedDec | Decrease Playback Speed | Play-at-Speed | ubcool2ic |
| 32 | PlaySpeedInc | Increase Playback Speed | Play-at-Speed | ubcool2ic |
| 33 | SetPlaySpeed | Adjust Playback Speed... | Play-at-Speed | ubcool2ic |
| 34 | InputDevice | Change Recording Device... | Device | ubcool2ic |
| 35 | OutputDevice | Change Playback Device... | Device | ubcool2ic |
| 36 | AudioHost | Change Audio Host... | Device | ubcool2ic |
| 37 | InputChannels | Change Recording Channels... | Device | ubcool2ic |
| 38 | SnapToOff | Snap-To Off | Selection | ubcool2ic |
| 39 | SnapToNearest | Snap-To Nearest | Selection | ubcool2ic |
| 40 | SnapToPrior | Snap-To Prior | Selection | ubcool2ic |
| 41 | SelStart | Selection to Start | Selection | ubcool2ic |
| 42 | SelEnd | Selection to End | Selection | ubcool2ic |
| 43 | SelExtLeft | Selection Extend Left | Selection | ubcool2ic |
| 44 | SelExtRight | Selection Extend Right | Selection | ubcool2ic |
| 45 | SelSetExtLeft | Set (or Extend) Left Selection | Selection | ubcool2ic |
| 46 | SelSetExtRight | Set (or Extend) Right Selection | Selection | BizyBot |
| 47 | SelCntrLeft | Selection Contract Left | Selection | BizyBot |
| 48 | SelCntrRight | Selection Contract Right | Selection | BizyBot |
| 49 | MoveToPrevLabel | Move to Previous Label | Selection | BizyBot |
| 50 | MoveToNextLabel | Move to Next Label | Selection | BizyBot |
| 51 | ExportMp3 | Export as MP3 | Export | BizyBot |
| 52 | ExportWav | Export as WAV | Export | BizyBot |
| 53 | ExportOgg | Export as OGG | Export | BizyBot |
| 54 | ExportFLAC | Export as FLAC | Export | BizyBot |
| 55 | ExportSel | Export Selected Audio... | Export | BizyBot |
| 56 | PrevFrame | Move Backward from Toolbars to Tracks | Focus | BizyBot |
| 57 | NextFrame | Move Forward from Toolbars to Tracks | Focus | BizyBot |
| 58 | PrevTrack | Move Focus to Previous Track | Focus | BizyBot |
| 59 | NextTrack | Move Focus to Next Track | Focus | BizyBot |
| 60 | FirstTrack | Move Focus to First Track | Focus | BizyBot |
| 61 | LastTrack | Move Focus to Last Track | Focus | BizyBot |
| 62 | ShiftUp | Move Focus to Previous and Select | Focus | BizyBot |
| 63 | ShiftDown | Move Focus to Next and Select | Focus | BizyBot |
| 64 | Toggle | Toggle Focused Track | Focus | BizyBot |
| 65 | ToggleAlt | Toggle Focused Track | Focus | BizyBot |
| 66 | CursorLeft | Cursor Left | Cursor | BizyBot |
| 67 | CursorRight | Cursor Right | Cursor | BizyBot |
| 68 | CursorShortJumpLeft | Cursor Short Jump Left | Cursor | BizyBot |
| 69 | CursorShortJumpRight | Cursor Short Jump Right | Cursor | BizyBot |
| 70 | CursorLongJumpLeft | Cursor Long Jump Left | Cursor | BizyBot |
| 71 | CursorLongJumpRight | Cursor Long Jump Right | Cursor | BizyBot |
| 72 | TrackPan | Change Pan on Focused Track... | Track | BizyBot |
| 73 | TrackPanLeft | Pan Left on Focused Track | Track | BizyBot |
| 74 | TrackPanRight | Pan Right on Focused Track | Track | BizyBot |
| 75 | TrackGain | Change Gain on Focused Track... | Track | BizyBot |
| 76 | TrackGainInc | Increase Gain on Focused Track | Track | BizyBot |
| 77 | TrackGainDec | Decrease Gain on Focused Track | Track | BizyBot |
| 78 | TrackMenu | Open Menu on Focused Track... | Track | BizyBot |
| 79 | TrackMute | Mute/Unmute Focused Track | Track | BizyBot |
| 80 | TrackSolo | Solo/Unsolo Focused Track | Track | BizyBot |
| 81 | TrackClose | Delete Focused Track | Track | BizyBot |
| 82 | TrackMoveUp | Move Focused Track Up | Track | BizyBot |
| 83 | TrackMoveDown | Move Focused Track Down | Track | BizyBot |
| 84 | TrackMoveTop | Move Focused Track to Top | Track | BizyBot |
| 85 | TrackMoveBottom | Move Focused Track to Bottom | Track | BizyBot |
| 86 | SetEnvelope | Set Envelope... | Scriptables I | BizyBot |
| 87 | SetProject | Set Project... | Scriptables I | BizyBot |
| 88 | Drag | Move Mouse... | Scriptables II | BizyBot |
| 89 | CompareAudio | Compare Audio... | Scriptables II | BizyBot |
