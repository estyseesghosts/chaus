# Bundled Geist Sans

chaus bundles Geist Sans for the Geist themes because Silvestris ships Geist
Mono but not Geist Sans, and deliberately does not force a UI family on its
consumers. Registration is local to `chaus-app`; `silvestris-fonts` is
untouched.

- Source: https://github.com/vercel/geist-font
- Commit: `ef2597cfc42f4797676df105517ccc0a1cfa6569`
- License: SIL Open Font License 1.1 (`OFL.txt` in this directory)
- Faces: Regular, Italic, Medium, SemiBold, Bold (static TTF)

SHA-256:

| File | SHA-256 |
| --- | --- |
| `Geist-Regular.ttf` | `85a1c6b18a6b0a06dfe9fd4f6d6a5d4979f74ec861eaef4bc7868b5492b8a117` |
| `Geist-Italic.ttf` | `be622eb7f66b89bd64575fca1436192019c923e2cd3cc9abe7506a9c50c9f5ba` |
| `Geist-Medium.ttf` | `3a3b36f0d0b981f4857f7f00eeef4a5ee123605575d362ab31ee7c19e3d11f2f` |
| `Geist-SemiBold.ttf` | `b9f52322d6d20a7ff1e7fe1310ff12ec851aebb83d24f8b4f1d4149298f489b2` |
| `Geist-Bold.ttf` | `3f21f02b827228c3f0c5fb230d027b5a1a263013618e8052bc1b3c5692812c88` |

The faces are compiled into the executable with `include_bytes!`; nothing is
downloaded or read at runtime. See `crates/chaus-app/src/fonts.rs`.
