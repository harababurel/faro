"""Coverage of the curated set, and the size of a subset woff2 per font."""
import io, json, pathlib, sys
from fontTools.ttLib import TTFont
from fontTools import subset

CORE = "".join(chr(c) for c in range(0x20, 0x7F)) + "".join(chr(c) for c in range(0xA0, 0x180))
EXTRA = "–—‘’“”•…·‹›←↑→↓↳≤≥⊘■▶▸▾◂○◌●✓✗✎❝�"
NEED = CORE + EXTRA
# Nice to have: what the renderer draws itself today (a fallback if it ever doesn't).
SHAPES = "─│┃╭╮╯╰▀▂▄▆█▌▎▐▖▗▘▙▚▛▜▝▞▟" + "".join(map(chr, [0x1FB82, 0x1FB85, 0x1FBE6, 0x1FBE7, 0x1FBEC, 0x1FBED, 0x1FBEE, 0x1FBEF, 0x1CD00, 0x1CDE5, 0xE0B4, 0xE0B6, 0x2800, 0x28FF]))
FONTS = {
    "JetBrains Mono": ["src/JetBrainsMono-Regular.woff2", "src/JetBrainsMono-Bold.woff2"],
    "Cascadia Mono": ["src/CascadiaMono-VF.ttf"],
    "Iosevka": ["src/Iosevka-400.ttf", "src/Iosevka-700.ttf"],
    "IBM Plex Mono": ["src/IBMPlexMono-Regular.ttf", "src/IBMPlexMono-Bold.ttf"],
    "Fira Code": ["src/FiraCode-VF.ttf"],
}
out = pathlib.Path("subset"); out.mkdir(exist_ok=True)
for name, files in FONTS.items():
    f = TTFont(files[0]); cmap = f.getBestCmap()
    missing = [c for c in NEED if ord(c) not in cmap]
    shapes = sum(ord(c) in cmap for c in SHAPES)
    adv = f["hmtx"]["M"][0] / f["head"].unitsPerEm if "M" in f.getGlyphOrder() else f["hmtx"][cmap[ord("M")]][0] / f["head"].unitsPerEm
    sizes = []
    for path in files:
        opts = subset.Options(); opts.flavor = "woff2"; opts.layout_features = ["kern", "liga", "calt"] if False else []
        opts.name_IDs = ["*"]; opts.notdef_outline = True; opts.hinting = False
        if path.endswith("VF.ttf"):
            # One weight from the variable font: instance it at 400 (and 700 later).
            from fontTools.varLib import instancer
            for wght in (400, 700):
                inst = instancer.instantiateVariableFont(TTFont(path), {"wght": wght})
                buf = io.BytesIO(); inst.save(buf); buf.seek(0)
                font = TTFont(buf)
                s = subset.Subsetter(opts); s.populate(text=NEED); s.subset(font)
                p = out / f"{name.replace(' ', '')}-{wght}.woff2"; font.flavor = "woff2"; font.save(p); sizes.append(p.stat().st_size)
        else:
            font = TTFont(path)
            s = subset.Subsetter(opts); s.populate(text=NEED); s.subset(font)
            p = out / f"{name.replace(' ', '')}-{'700' if ('Bold' in path or '700' in path) else '400'}.woff2"; font.flavor = "woff2"; font.save(p); sizes.append(p.stat().st_size)
    print(f"{name:15} advance {adv:.3f} em  missing {len(missing):2} {''.join(missing)!r:20}  shapes-in-font {shapes}/{len(SHAPES)}  subset woff2 " + " + ".join(f"{s/1000:.1f}" for s in sizes) + f" = {sum(sizes)/1000:.1f} KB")
