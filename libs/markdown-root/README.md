# libs/markdown-root

A declared markdown root, and the only way to get a file out of it. Spine-owned
shared code with no domain of its own — see
[Three architectural nouns](../../README.md#three-architectural-nouns).

Two capabilities read markdown out of a knowledge store the operator points them
at. Scouting resolves opportunity notes and interest profiles; calendar imports
event notes. Both take a root path from the private overlay plus a glob relative
to it, and both have to answer the same question before reading anything: is
this file actually inside the root that was declared?

## Why it exists

The answer used to be "assume so". `capabilities/scouting/src/score.rs` and
`capabilities/scouting/src/sources/obsidian_md.rs` each carried their own
`root.join(pattern)` resolver. They had already diverged — one handled an exact
file, the other only a directory — and neither checked containment, so a pattern
of `../../.ssh` resolved happily and a symlink pointing out of the vault was
followed without comment. Calendar's markdown importer would have been the third
copy of that.

That is the repo's stated extraction rule met exactly: the same shape in more
than one place, diverging, in a way no one decided.

## What the type guarantees

- A `MarkdownRoot` exists only for a directory that exists, canonicalized once.
- Every path handed back is inside that directory, proven **after** symlink
  resolution rather than by string prefix.
- A pattern containing `..` or an absolute path is refused before any read, so
  no evidence of what lies outside the root leaks through an error message.
- Anything that would escape is an error naming the offending path — never a
  quietly dropped file. An operator cannot fix a config the resolver hides.
- `relative_id()` returns a slash-separated, root-anchored identity, so the
  `(source, external_id)` a note is imported under does not depend on which
  machine imported it.

## What it deliberately is not

**Not a glob engine.** The patterns in play are `Some/Dir/*.md` and one exact
`Some/File.md`, which is all the config shape has ever declared. A real matcher
would be a dependency, and this crate is std-only so that folding it into a
consumer never changes that consumer's dependency resolution.

**Not a parser.** What a markdown file *means* — frontmatter, an event, an
interest profile — belongs to the capability that declared the root.

**Not tilde expansion.** `axon_config::expand_tilde` owns that; the caller
applies it before declaring a root.
