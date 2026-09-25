# TODO

Running list of things we want to do. Details live in the linked devlogs.

- [ ] **GPU video decode.** Try NVDEC hardware decode to speed up video processing. See
  [AnalyzerPerformance.md §4d](devlog/AnalyzerPerformance.md) ("Server GPU decode (NVDEC)"),
  and measure CPU cost per video first as described in §5.
- [ ] **Richer GUI log text from the pokedex.** Use the `pokedex/` database to add details
  (names, types, moves, etc.) to the transcript lines shown in the web GUI's logs.
