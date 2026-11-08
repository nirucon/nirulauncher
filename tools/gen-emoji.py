#!/usr/bin/env python3
"""Build data/emoji.tsv from Unicode emoji-test.txt and CLDR annotations.

Usage: tools/gen-emoji.py SRC_DIR > data/emoji.tsv

SRC_DIR holds the downloaded inputs (they are not kept in the repository):
  emoji-test.txt   https://unicode.org/Public/emoji/16.0/emoji-test.txt
  ann-en.xml       cldr release-46 common/annotations/en.xml
  ann-sv.xml       cldr release-46 common/annotations/sv.xml
  der-en.xml       cldr release-46 common/annotationsDerived/en.xml
  der-sv.xml       cldr release-46 common/annotationsDerived/sv.xml

Output, one row per emoji or symbol, tab separated:
  group, characters, English name, Swedish name, keywords (en and sv,
  "|" separated), skin-tone variants (five sequences, space separated, or
  empty).
Skin-tone and hair components are not rows of their own: Tab in the picker
cycles the tone of the rows that have variants.
"""
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

GROUPS = {
    "Smileys & Emotion": "Smileys",
    "People & Body": "People",
    "Animals & Nature": "Nature",
    "Food & Drink": "Food",
    "Travel & Places": "Travel",
    "Activities": "Activities",
    "Objects": "Objects",
    "Symbols": "Emoji symbols",
    "Flags": "Flags",
}
TONES = ["\U0001F3FB", "\U0001F3FC", "\U0001F3FD", "\U0001F3FE", "\U0001F3FF"]

# Typographic and technical symbols, drawn with the text font.
# (characters, English name, Swedish name, extra keywords)
SYMBOLS = [
    ("©", "copyright sign", "upphovsrättstecken", "copyright c upphovsrätt"),
    ("®", "registered sign", "registrerat varumärke", "registered r trademark varumärke"),
    ("™", "trade mark sign", "varumärke", "trademark tm"),
    ("℗", "sound recording copyright", "fonogram", "phonogram p"),
    ("°", "degree sign", "gradtecken", "degree grader temperatur"),
    ("℃", "degree celsius", "grader celsius", "celsius temperatur"),
    ("℉", "degree fahrenheit", "grader fahrenheit", "fahrenheit"),
    ("±", "plus-minus sign", "plus minus", "plusminus"),
    ("×", "multiplication sign", "gångertecken", "times multiply gånger"),
    ("÷", "division sign", "divisionstecken", "divide delat"),
    ("−", "minus sign", "minustecken", "minus"),
    ("≈", "almost equal to", "ungefär lika med", "approximately ungefär"),
    ("≠", "not equal to", "inte lika med", "not equal"),
    ("≤", "less-than or equal to", "mindre än eller lika med", "less equal"),
    ("≥", "greater-than or equal to", "större än eller lika med", "greater equal"),
    ("∞", "infinity", "oändlighet", "infinite oändligt"),
    ("√", "square root", "kvadratrot", "root rot"),
    ("∑", "n-ary summation", "summa", "sum sigma"),
    ("π", "greek small letter pi", "pi", "pi"),
    ("µ", "micro sign", "mikro", "micro my"),
    ("‰", "per mille sign", "promille", "permille promille"),
    ("½", "vulgar fraction one half", "en halv", "half halv"),
    ("¼", "vulgar fraction one quarter", "en fjärdedel", "quarter fjärdedel"),
    ("¾", "vulgar fraction three quarters", "tre fjärdedelar", "three quarters"),
    ("¹", "superscript one", "upphöjd etta", "superscript upphöjd 1"),
    ("²", "superscript two", "upphöjd tvåa", "squared kvadrat upphöjd 2"),
    ("³", "superscript three", "upphöjd trea", "cubed kubik upphöjd 3"),
    ("→", "rightwards arrow", "pil höger", "arrow right pil"),
    ("←", "leftwards arrow", "pil vänster", "arrow left pil"),
    ("↑", "upwards arrow", "pil upp", "arrow up pil"),
    ("↓", "downwards arrow", "pil ner", "arrow down pil"),
    ("↔", "left right arrow", "pil vänster höger", "arrow both pil"),
    ("↕", "up down arrow", "pil upp ner", "arrow vertical pil"),
    ("⇒", "rightwards double arrow", "dubbelpil höger", "implies arrow pil medför"),
    ("⇐", "leftwards double arrow", "dubbelpil vänster", "arrow pil"),
    ("⇔", "left right double arrow", "dubbelpil vänster höger", "iff equivalent arrow pil"),
    ("↵", "downwards arrow with corner leftwards", "returpil", "return enter arrow pil"),
    ("⌘", "place of interest sign", "kommandotangent", "command cmd mac"),
    ("⌥", "option key", "optionstangent", "option alt mac"),
    ("⇧", "upwards white arrow", "skifttangent", "shift arrow"),
    ("⌫", "erase to the left", "backsteg", "backspace delete radera"),
    ("€", "euro sign", "euro", "currency valuta eur pengar"),
    ("£", "pound sign", "pund", "currency valuta gbp sterling"),
    ("$", "dollar sign", "dollar", "currency valuta usd"),
    ("¢", "cent sign", "cent", "currency valuta"),
    ("¥", "yen sign", "yen", "currency valuta jpy yuan"),
    ("₹", "indian rupee sign", "indisk rupie", "currency valuta inr rupee"),
    ("₽", "ruble sign", "rubel", "currency valuta rub"),
    ("₩", "won sign", "won", "currency valuta krw"),
    ("₺", "turkish lira sign", "turkisk lira", "currency valuta try"),
    ("₿", "bitcoin sign", "bitcoin", "currency valuta btc crypto"),
    ("¤", "currency sign", "valutatecken", "currency valuta"),
    ("kr", "Swedish krona", "svensk krona", "currency valuta sek kronor"),
    ("“", "left double quotation mark", "vänster dubbelt citattecken", "quote citat"),
    ("”", "right double quotation mark", "höger dubbelt citattecken", "quote citat svenska"),
    ("‘", "left single quotation mark", "vänster enkelt citattecken", "quote citat apostrophe"),
    ("’", "right single quotation mark", "höger enkelt citattecken", "quote citat apostrof apostrophe"),
    ("„", "double low-9 quotation mark", "nedre dubbelt citattecken", "quote citat german"),
    ("‚", "single low-9 quotation mark", "nedre enkelt citattecken", "quote citat"),
    ("«", "left-pointing double angle quotation mark", "vänster gåsögon", "guillemet quote citat"),
    ("»", "right-pointing double angle quotation mark", "höger gåsögon", "guillemet quote citat"),
    ("‹", "single left-pointing angle quotation mark", "vänster enkel gåsögon", "guillemet quote"),
    ("›", "single right-pointing angle quotation mark", "höger enkel gåsögon", "guillemet quote"),
    ("–", "en dash", "kort tankstreck", "dash streck tankstreck"),
    ("—", "em dash", "långt tankstreck", "dash streck tankstreck"),
    ("…", "horizontal ellipsis", "ellips", "dots ellipsis punkter"),
    ("•", "bullet", "punkt", "bullet list lista"),
    ("·", "middle dot", "mittpunkt", "dot interpunct punkt"),
    ("§", "section sign", "paragraftecken", "section paragraf"),
    ("¶", "pilcrow sign", "stycketecken", "paragraph stycke"),
    ("†", "dagger", "kors", "dagger obelisk"),
    ("‡", "double dagger", "dubbelkors", "dagger"),
    ("№", "numero sign", "nummertecken", "number nummer"),
    ("✓", "check mark", "bock", "check tick ok bock"),
    ("✗", "ballot x", "kryss", "cross x fel kryss"),
    ("★", "black star", "stjärna", "star stjärna"),
    ("☆", "white star", "vit stjärna", "star stjärna"),
    ("♥", "black heart suit", "hjärter", "heart hjärta"),
    ("☐", "ballot box", "ruta", "box checkbox kryssruta"),
    ("☑", "ballot box with check", "kryssad ruta", "checkbox done klar"),
    ("‽", "interrobang", "interrobang", "question exclamation"),
    ("¿", "inverted question mark", "omvänt frågetecken", "spanish question fråga"),
    ("¡", "inverted exclamation mark", "omvänt utropstecken", "spanish exclamation"),
    (" ", "no-break space", "hårt mellanslag", "nbsp space mellanslag"),
    (" ", "thin space", "smalt mellanslag", "space mellanslag"),
]


def annotations(path):
    names, words = {}, {}
    root = ET.parse(path).getroot()
    for a in root.iter("annotation"):
        cp = a.get("cp")
        text = (a.text or "").strip()
        if not cp or not text:
            continue
        if a.get("type") == "tts":
            names[cp] = text
        else:
            words[cp] = [w.strip() for w in text.split("|") if w.strip()]
    return names, words


def strip_vs(s):
    return s.replace("️", "")


def clean(s):
    return s.replace("\t", " ").replace("\n", " ").strip()


def main():
    src = Path(sys.argv[1])
    en_n, en_w = annotations(src / "ann-en.xml")
    sv_n, sv_w = annotations(src / "ann-sv.xml")
    for path, n, w in (("der-en.xml", en_n, en_w), ("der-sv.xml", sv_n, sv_w)):
        dn, dw = annotations(src / path)
        for k, v in dn.items():
            n.setdefault(k, v)
        for k, v in dw.items():
            w.setdefault(k, v)

    rows = []  # (group, chars, test_name)
    group = None
    for line in (src / "emoji-test.txt").read_text(encoding="utf-8").splitlines():
        if line.startswith("# group:"):
            group = line.split(":", 1)[1].strip()
            continue
        if not line or line.startswith("#"):
            continue
        codes, rest = line.split(";", 1)
        status, comment = rest.split("#", 1)
        if status.strip() != "fully-qualified":
            continue
        chars = "".join(chr(int(c, 16)) for c in codes.split())
        name = comment.strip().split(" ", 2)[2] if comment.strip().count(" ") >= 2 else ""
        rows.append((group, chars, name))

    bases, variants = [], {}
    for group, chars, name in rows:
        if group == "Component":
            continue
        if any(t in chars for t in TONES):
            base = chars
            for t in TONES:
                base = base.replace(t, "")
            tones = {i for i, t in enumerate(TONES) if t in chars}
            if len(tones) == 1:
                variants.setdefault(strip_vs(base), {})[tones.pop()] = chars
            continue
        bases.append((group, chars, name))

    def name_of(table, chars):
        return table.get(chars) or table.get(strip_vs(chars)) or ""

    out = sys.stdout
    seen = set()
    for group, chars, test_name in bases:
        key = strip_vs(chars)
        if key in seen:
            continue
        seen.add(key)
        en = name_of(en_n, chars) or test_name
        sv = name_of(sv_n, chars)
        words = []
        for w in (en_w.get(chars) or en_w.get(key) or []) + (sv_w.get(chars) or sv_w.get(key) or []):
            w = clean(w).lower()
            if w and w not in words and w != en.lower() and w != sv.lower():
                words.append(w)
        tone = variants.get(key, {})
        tone_text = " ".join(tone[i] for i in range(5)) if len(tone) == 5 else ""
        out.write("\t".join([GROUPS[group], chars, clean(en), clean(sv), "|".join(words), tone_text]) + "\n")
    for chars, en, sv, words in SYMBOLS:
        out.write("\t".join(["Text symbols", chars, en, sv, "|".join(words.split()), ""]) + "\n")


if __name__ == "__main__":
    main()
