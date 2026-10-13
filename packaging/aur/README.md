# AUR packages

Three packages, none uploaded yet (they need the GitHub repo public, D86):

| Package | What |
|---|---|
| `faro-git` | latest commit, built from source |
| `faro` | the tagged release, built from source (system OpenSSL) |
| `faro-bin` | the release's prebuilt dist archives (statically linked OpenSSL, D133) |

All three `provides`/`conflicts` `faro`. The `faro` and `faro-bin` PKGBUILDs
are pinned to a `pkgver` with `sha256sums=('SKIP')`; nothing works until the
`v<pkgver>` tag and its release exist.

## Release steps (owner)

1. Tag and push the release; wait for `release.yml` to publish the GitHub
   Release (archives + Homebrew formula).
2. In `faro/` and `faro-bin/`: bump `pkgver` if new, then
   `updpkgsums && makepkg --printsrcinfo > .SRCINFO`.
3. Test: `makepkg -si` then `namcap` on the built package (the
   `ld-linux-x86-64.so.2` warning is the usual false alarm, D86).
4. Push to the AUR with the `~/.ssh/aur` key:
   `git push ssh://git@aur.archlinux.org/<pkgname>.git master`
   (init the repo from the PKGBUILD + .SRCINFO; never push before the
   checksums are real).

`faro-git` needs no checksums (its `pkgver()` tracks the clone).
