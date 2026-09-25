### Changed

- Session admission records rejected connect attempts through a shared helper instead of a local macro; metric behavior is unchanged.
- The session admission test for unpolled cancellation now waits on a transport notification instead of spinning up to 64 scheduler yields, so the reclaim assertion cannot time out under load.