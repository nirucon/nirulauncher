# Intent aliases

The launcher matches exact English and Swedish phrases after normalizing case,
whitespace and punctuation. A phrase never becomes a shell command; it resolves
to a fixed action in the launcher catalogue.

| Action | English examples | Swedish aliases |
|---|---|---|
| Lock | `lock`, `lock screen` | `lås`, `lås skärmen` |
| Log out | `logout` | `logga ut` |
| Reboot | `reboot` | `starta om` |
| Shut down | `shutdown` | `stäng av` |
| Focus mode | `focus mode` | `fokusläge` |
| Presentation mode | `presentation mode` | `mötesläge` |
| Stop context | `stop context` | `avsluta läge` |
| Do Not Disturb | `dnd`, `do not disturb` | `stör ej` |
| Keep awake | `keep awake` | `håll vaken` |
| Volume up | `volume up` | `höj volymen` |
| Volume down | `volume down` | `sänk volymen` |
| Mute output | `mute` | `ljud av` |
| Mute microphone | `mic mute` | `mikrofon av` |
| Brightness up | `brightness up` | `ljusare` |
| Brightness down | `brightness down` | `mörkare` |
| Night light | `night light` | `nattljus` |
| Window swallowing | `window swallowing`, `swallow` | `fönstersväljning`, `svälj fönster` |
| Restore windows | `bring off-screen windows back` | `hämta tillbaka fönster` |
| Wi-Fi on | `wifi on` | `slå på wifi` |
| Wi-Fi off | `wifi off` | `stäng av wifi` |

The implemented table is authoritative in `src/intent.rs`.

A query that starts with `:` (`:heart`, `:tummen`) adds the best eight emoji
and symbols as the first rows; Enter pastes the selected one (see the emoji
picker in [GUIDE.md](GUIDE.md)).

A query that starts with `/` (`/ faktura`, `/kvitto 2024`) searches files in
the home folder instead; Enter opens the selected file (see file search in
[GUIDE.md](GUIDE.md)).
