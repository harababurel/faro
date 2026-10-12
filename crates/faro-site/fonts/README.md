# Fonts for the OG image rasteriser

`JetBrainsMono-Regular.ttf` and `JetBrainsMono-Bold.ttf` are the same
JetBrains Mono subsets the web demo ships (D107), decompiled from woff2
because `resvg` can't read woff2. `scene.rs` embeds them with
`include_bytes!` and loads only these into resvg's font database, so the
OG image comes out the same on every machine — no system fonts.

`OFL.txt` is their licence (SIL Open Font License 1.1), the same file the
web demo and the site template carry.

Regenerate after changing the webfont subsets (needs Python 3 with
fontTools and brotli):

```console
$ python3 -c "from fontTools.ttLib import TTFont; f=TTFont('crates/faro-web/www/fonts/JetBrainsMono-Regular.woff2'); f.flavor=None; f.save('crates/faro-site/fonts/JetBrainsMono-Regular.ttf')"
```

…and the same line with `Bold` in place of `Regular`.
