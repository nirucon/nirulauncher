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
| Restore windows | `bring off-screen windows back` | `hämta tillbaka fönster` |
| Wi-Fi on | `wifi on` | `slå på wifi` |
| Wi-Fi off | `wifi off` | `stäng av wifi` |

The implemented table is authoritative in `src/intent.rs`.
