# Fonts for the OG image and the phone-frame outlines

`JetBrainsMono-Regular.ttf`, `JetBrainsMono-Bold.ttf` and
`JetBrainsMono-Italic.ttf` are the same JetBrains Mono subsets the web
demo ships (D107), decompiled from woff2 because `resvg` can't read
woff2. `scene/mod.rs` embeds all three with `include_bytes!`.

The OG rasteriser loads only Regular and Bold into resvg's font
database, so the OG image comes out the same on every machine — no
system fonts. The Italic subset is used only by the phone frames'
outliner (`scene/phone_font.rs`), which draws italic runs with it; the
OG bytes never see it.

`OFL.txt` is their licence (SIL Open Font License 1.1), the same file
the web demo and the site template carry.

Regenerate after changing the webfont subsets (needs Python 3 with
fontTools and brotli):

```console
$ python3 -c "from fontTools.ttLib import TTFont; f=TTFont('crates/faro-web/www/fonts/JetBrainsMono-Regular.woff2'); f.flavor=None; f.save('crates/faro-site/fonts/JetBrainsMono-Regular.ttf')"
```

…and the same line with `Bold` or `Italic` in place of `Regular`.

After regenerating, check the name metadata with fontTools, since the
outliner picks faces by these names:

```console
$ python3 -c "from fontTools.ttLib import TTFont; f=TTFont('crates/faro-site/fonts/JetBrainsMono-Italic.ttf'); print(f['name'].getDebugName(1), '/', f['name'].getDebugName(2), '/', f['OS/2'].usWeightClass)"
JetBrains Mono / Italic / 400
```

Verified on the current files: family `JetBrains Mono`, subfamilies
Regular / Bold / Italic, weights 400 / 558 / 400.
