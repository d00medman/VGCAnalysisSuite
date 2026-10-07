# Specs

This project uses spec-driven development. The layout follows GitHub's Spec Kit, kept by
hand and without its tooling: Spec Kit creates a branch per feature, which would clash with
the shared working tree (see `CLAUDE.md`).

```
specs/
  vision.md             what the product is, who it's for, its principles and roadmap
  _templates/           copy these to start a feature
  NNN-feature-name/
    spec.md             what and why: behaviour, scope, acceptance criteria, open questions
    plan.md             how: design, data model, API, migration number, risks
    tasks.md            ordered, checkable steps
```

`vision.md` plays the part of Spec Kit's constitution. Its §6 principles apply to every
feature.

## When a feature needs a spec

- **Needs one:** anything that changes the schema or battle data, changes how the GUI
  behaves, or adds a pipeline stage.
- **Doesn't:** bug fixes, small tweaks, and refactors that don't change behaviour.

## The loop

1. **Specify.** Claude drafts `spec.md` from a sentence or a TODO item, after reading the
   relevant code. It names the part of the vision the feature serves.
2. **Clarify.** Claude lists the questions the code can't answer, and the user answers
   them. Repeat until `spec.md` has no blocking questions left. The user approves it.
3. **Plan.** Claude writes `plan.md`. A schema change claims its migration number here,
   and tells other sessions which number it took. The user reviews the plan.
4. **Tasks.** Claude writes `tasks.md`: small steps, in order, each one checkable.
5. **Implement.** If the work has to diverge from the spec or plan, Claude stops, updates
   the document and flags the change to the user. It never diverges quietly.
6. **Converge.** Claude checks each acceptance criterion against the code and reports any
   gaps. The user runs the checks that need real recordings or the real GUI.
7. **Close.** Set the spec's status to Done. The folder stays as the record of what the
   feature is.

The research and history behind decisions stays in `devlog/`. Specs link to it rather than
repeat it.

## Status line

Every `spec.md` starts with one status:

- `Draft`
- `Approved`
- `In progress`
- `Done`
- `Parked`: worth doing, deliberately set aside. The spec says why, and what would bring
  it back.
- `Dropped`

## Numbering

Number features in the order they're started, not by priority. Take the next unused number.
