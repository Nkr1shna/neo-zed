# Release Identity

Canonical release-facing identity map for this mission.

## Stable names

| Surface | Value |
| --- | --- |
| Product name | `Neo Zed` |
| CLI command | `neozed` |
| URL scheme | `neozed://` |
| Base app ID | `dev.neozed` |
| Domain | `https://neozed.dev` |
| API domain | `https://api.neozed.dev` |
| Cloud/update domain | `https://cloud.neozed.dev` |
| Repo URL | `https://github.com/Nkr1shna/neo-zed/` |

## Channel mapping

| Channel | Display name | Bundle/app ID |
| --- | --- | --- |
| stable | `Neo Zed` | `dev.neozed` |
| preview | `Neo Zed Preview` | `dev.neozed.Preview` |
| nightly | `Neo Zed Nightly` | `dev.neozed.Nightly` |
| dev | `Neo Zed Dev` | `dev.neozed.Dev` |

## Worker guidance

- Prefer deriving per-platform identifiers from the shared channel mapping instead of inventing custom spellings per file.
- Update release-facing Zed references even if they are not exact string replacements.
- Preserve internal crate/type/module names when they are not user-visible.
- Leave intentionally compatibility-sensitive internal protocol names alone unless the surface is public registration metadata or public documentation.
