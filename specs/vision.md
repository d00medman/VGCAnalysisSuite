# Vision

Rev 1 · 2026-10-02 · Draft for review.

The top-level spec. Every feature spec in `specs/` names the part of this document it serves.
If a feature spec turns out to contradict this one, fix this one first.

`Decided:` marks things agreed with the user. `Open:` marks questions still to settle.
Everything else is a working assumption.

---

## 1. Problem

Pokémon Champions gives players no access to their own history. There's no match list, no
game count and no replays, and the developers have said in an interview that replay
functionality isn't coming. Players who want to review or improve have nothing to work with.

## 2. What we're building

A website where Champions players upload recordings of their battles, get a transcript of
each battle, and gain insights about their play. It bridges the gap the game leaves.

- **Decided: the website comes first.** It should work in desktop and mobile browsers. A
  native app may come later.
- **Decided: multi-user from day one.** Every battle, team and Pokémon belongs to an
  account, even while the user is the only one.

## 3. Who it's for

1. **The user, first.** Phase 1 is a product the user likes and uses on their own games.
2. **The Champions / VGC community, next.** The site is deployed to AWS and advertised to
   players.
3. **Then it evolves with what its users want.**

## 4. Core concepts

How the pieces relate. Feature specs must agree with this model, or change it here first.

- **Trainer:** an account. Owns everything below.
- **Recording:** a video file the trainer uploads. Sources (see §5):
  - **Decided:** mobile screen recordings, on both iOS and Android. All current test data
    is from iOS.
  - **Decided:** capture card recordings (Switch).
  - **Decided:** Switch 2 built-in captures. These are capped at 30 seconds, so one battle
    spans many clips.
- **Battle:** one game. Built from one recording, or from several recordings that the
  trainer stitches together by hand (**decided**). Has a transcript, a result, and the team
  the trainer brought.
- **Transcript:** the battle's event log, extracted from the video's message text. This is
  the core of the product.
- **My Pokémon:** a Pokémon the trainer has built, with species, moves, ability, item, SP
  spread and nature. It's user-entered data that can't be rebuilt.
- **Team:** a set of My Pokémon. **Decided:** one Pokémon can be in several teams, so teams
  reference Pokémon and don't copy them.
- **Battle ↔ team:** team preview shows which Pokémon the trainer brought.
  - Processing team preview isn't built yet.
  - Until it is, the transcript tells us which 4 Pokémon the trainer used.
- **Pokedex:** reference data for species, moves, types and regulations. It's rebuilt from
  public sources, so it is replaceable, unlike everything above.

## 5. Capabilities

In rough priority order. Pillar 1 is the product; the others make it worth coming back to.

1. **Upload → transcript.** Upload one or more recordings, assemble them into a battle,
   and get its transcript.
2. **My Pokémon and teams.** Enter Pokémon by hand, and group them into teams.
   - **Open:** importing teams from rental codes. What's possible depends on whether a code
     can be resolved to a team directly (unlikely) or the team has to be read from
     screenshots. The user can supply data.
3. **Stats.** A stats page over the trainer's battles. It starts simple, such as wins and
   losses, and the user adds more advanced stats over time.
4. **Derived data.** Facts inferred from transcripts and stored against the battle: which
   Pokémon appeared, the moves used, and what those reveal about the opponent. This is what
   advanced stats will be built on.
5. **Tools.** For example the damage calculator, filled in with the trainer's own Pokémon.

## 6. Principles

- **Irreplaceable data is protected.** Battles, hand-set results, Pokémon and teams are
  backed up (`scripts/backup-battles.sh`). Schema changes to them follow
  `devlog/SchemaIteration.md`.
- **Reference data is rebuildable.** The pokedex can be thrown away and rebuilt.
- **Privacy by default.** Originals are deleted after processing, and previews are redacted
  and silent. See `devlog/DeploymentPlan.md` §2–4.
  - **Decided:** notification redaction must cover Android as well as iOS. The deployment
    plan only covers iOS banners so far.
- **Hand correction beats silent failure.** Where the video can't tell us something, like
  the result after a forfeit, the trainer can set it by hand.

## 7. Non-goals

**Decided:**

- No live overlay or assistance during a battle.
- No ladder, matchmaking or tournament features.
- No social feed.
- Scouting opponents' public teams through rental codes isn't a goal for now.

## 8. Phases

1. **Phase 1: a product the user likes.**
   - **Decided:** there's no fixed finish line. The user keeps filling gaps in the
     capabilities above until it's something they want to use.
   - Phase 1 ends when the user decides it does.
2. **Phase 2: deployed.** On AWS, as in `devlog/DeploymentPlan.md`.
3. **Phase 3: community.** Advertised to players; it evolves with their feedback.

## 9. Roadmap

How current work maps onto the capabilities. Each item gets a feature spec
(`specs/NNN-name/`) when we pick it up.

### Stages

Ordered by dependency. Stage D can run alongside B and C.

1. **A: Foundation**
   - Trainer table → `001-trainer-table`
2. **B: First stats**
   - Mark a battle's result by hand
   - Stats page with wins and losses
3. **C: Your Pokémon**
   - My Pokémon and teams
   - Damage calculator
   - Rental code import, once its open question is settled
4. **D: More inputs**
   - Stitch recordings into one battle
   - Android, capture card and Switch 2 inputs. Switch 2 needs stitching first.
   - Transcribe Batch 1 to check the analyzer on newer footage
5. **E: Data inferred from transcripts**
   - Store inferred facts
   - Team preview processing
   - Richer GUI log text
   - Forfeit detection
   - Advanced stats
6. **Optional:** sprites, then on-screen Pokémon recognition.
7. **Dev tooling, any time:** per-branch Docker stacks. Lets parallel sessions, or the
   user, run several branches side by side, each with its own copy of the data.
8. **Phase 2:** redaction, the job queue, auth and infrastructure
   (`devlog/DeploymentPlan.md`). Also GPU decode, if the numbers justify it.

The critical path to a usable product: **trainer table → hand-set result → wins/losses
page.**

### All items

Arrows show dependencies.

| Item | Capability | Status |
|---|---|---|
| Transcript extraction (analyzer, web GUI) | 1 | Working |
| Trainer table (accounts) | foundation for 2–5 | In progress, `001-trainer-table` |
| Stitch recordings into one battle | 1 | New |
| Android, capture card and Switch 2 inputs | 1 | New |
| Notification redaction (iOS, then Android) | phase 2 | Planned, `DeploymentPlan.md` §4 |
| Mark a battle's result by hand → detect forfeit from video | 1, 3 | TODO |
| My Pokémon + teams (trainer table →) | 2 | TODO |
| Team preview processing | 2, 4 | New |
| Rental code import | 2 | Open, see §5 |
| Stats page: wins/losses (result →) | 3 | New |
| Store data inferred from transcripts | 4 | TODO |
| Richer GUI log text from the pokedex | 1 | TODO |
| Sprite data → on-screen Pokémon recognition | 1, 4 | TODO, optional |
| Damage calculator (teams →) | 5 | TODO |
| GPU decode | performance | TODO |
| Per-branch Docker stacks | dev tooling | TODO |
| Deployment | phase 2 | Planned, `devlog/DeploymentPlan.md` |

## 10. Open questions

- **Rental codes** (§5): can a code be resolved directly, or must we read screenshots?
- **Stitching:** can the trainer stitch clips, or reorder them, after the transcript has
  run? Or only before?
- **Battles without a team:** what do we show for a battle where the trainer hasn't logged
  the team they brought? Do we match the transcript against their saved Pokémon?
- **Android recordings:** where do we get samples? Android notification banners differ by
  manufacturer and OS version, so a fixed blur band may not be enough. Android recordings
  may also differ from iOS in resolution, aspect ratio and frame rate, which the analyzer
  would need to handle.
- **Native app:** what would it add over the mobile website, and when would it be worth it?
