# Inter, the body face on Linux

`Inter-Latin.ttf` is [Inter](https://github.com/rsms/inter) 4.1 (SIL Open Font License 1.1,
`OFL.txt`), cut down to what featherplay's body text needs on Linux, where there's no Segoe UI
Variable. Only the Linux build embeds it (`src/fonts.rs`); the Windows build doesn't.

What's kept:

- the weight axis (100 to 900); the optical-size axis is pinned at 14, its default, the size
  the body text is drawn at (the renderer never sets that axis);
- Latin, Latin-1 and Latin Extended-A, the Extended-B letters of Romanian, Vietnamese and
  Azerbaijani (and ƒ), the spacing modifiers and combining marks, and Vietnamese's
  precomposed letters;
- General Punctuation, the currency signs, and the few symbols the UI writes (arrows, minus,
  numero, trade mark, ≠ ≤ ≥ ≈ ∞);
- the default OpenType features (kerning, ligatures, contextual alternates, marks), and every
  name record (the copyright and licence lines included); no hinting.

Anything else (Greek, Cyrillic, other scripts, emoji) goes to the fonts fontconfig finds.

What's changed: the line metrics are Segoe UI Variable's, so text sits in its line and lines
are as tall as on Windows, where the UI was designed and tested. Both fonts have 2,048 units
per em, so the numbers are copied as they are. They were read (and only read) from
`SegUIVar.ttf`, Segoe UI Variable version 2.03 as Windows 11 ships it, on 2026-11-12; nothing
else of Segoe's is in this file.

| Table | Field | Inter 4.1 | Here (Segoe UI Variable's) |
|---|---|---|---|
| `hhea` | ascender, descender, line gap | 1984, −494, 0 | 2210, −514, 0 |
| `OS/2` | typo ascender, descender, line gap | 1984, −494, 0 | 1491, −431, 269 |
| `OS/2` | win ascent, descent | 1984, 494 | 2210, 514 |
| `OS/2` | `fsSelection` bit 7, USE_TYPO_METRICS | set | clear, as Segoe has it |

With that bit clear the renderer goes by `hhea`: 1.079 em above the baseline and 0.251 below,
a line of 1.33 em, where Inter's own is 1.21 em.

## Making it

From `Inter-4.1.zip` on the release page
(<https://github.com/rsms/inter/releases/tag/v4.1>, SHA-256
`9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e`), take `InterVariable.ttf`
and `LICENSE.txt` (copied here as `OFL.txt`). Then, with fonttools 4.60.1
(`pip install fonttools==4.60.1`):

```sh
export SOURCE_DATE_EPOCH=1731628800  # the release's date, for the head table's timestamp
python -m fontTools.varLib.instancer InterVariable.ttf opsz=14 -o Inter-opsz14.ttf
python -m fontTools.subset Inter-opsz14.ttf \
  --unicodes="U+0000-017F,U+018F,U+0192,U+01A0-01A1,U+01AF-01B0,U+0218-021B,U+0237,U+0259,U+02B0-02FF,U+0300-036F,U+1E9E,U+1EA0-1EF9,U+2000-206F,U+20A0-20C0,U+2113,U+2116,U+2122,U+2126,U+212E,U+2190-2193,U+2212,U+2215,U+2219,U+221E,U+2248,U+2260,U+2264-2265,U+FEFF,U+FFFD" \
  --no-hinting --name-IDs="*" --output-file=Inter-subset.ttf
python -c "
from fontTools.ttLib import TTFont
f = TTFont('Inter-subset.ttf')
h, o = f['hhea'], f['OS/2']
h.ascent, h.descent, h.lineGap = 2210, -514, 0
o.sTypoAscender, o.sTypoDescender, o.sTypoLineGap = 1491, -431, 269
o.usWinAscent, o.usWinDescent = 2210, 514
o.fsSelection &= ~(1 << 7)
f.save('Inter-Latin.ttf')
"
```

That gives the same file byte for byte: 206,772 bytes, SHA-256
`37d7141909978b8496723113e5ce6ada56833820f329c7bc5a760e4b2842d11a`.
