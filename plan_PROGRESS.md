# Progress: plan

Started: za 24 jan 2026  0:09:41 CET

## Status

IN_PROGRESS

## Task List

Based on the plan file findings:

- [x] Task 1: Investigate why DownloadComplete event doesn't fire (check src/lib.rs for Event::DownloadComplete emission)
- [x] Task 2: Determine if post-processing (PAR2 verification, extraction) is blocking completion
- [x] Task 3: Fix the DownloadComplete event firing issue so speedtest doesn't need 99% threshold workaround
- [ ] Task 4: Verify the fix by running the speedtest and confirming DownloadComplete fires

## Tasks Completed

### Task 1: Investigation Complete
**Root Cause Identified**: Event::DownloadComplete IS being sent (at multiple locations: lines 3145, 3593, 3935, 4170), but post-processing was NEVER automatically triggered afterward. This is confirmed by TODO comments at lines 650-657 indicating this was planned for "Phase 2" but never implemented.

The issue is NOT that the event doesn't fire - it fires correctly. The issue is that the download flow stops after sending the event, without starting post-processing.

### Task 2: Post-Processing Analysis Complete
**Finding**: Post-processing does NOT block completion - it was simply never being triggered automatically. The library design emits Event::DownloadComplete but requires external consumers to manually call `start_post_processing()`.

### Task 3: Fix Implemented
**Changes Made** to automatically trigger post-processing after download completion:

1. **src/lib.rs:3032-3041** - Added `downloader` clone in `start_queue_processor()`
2. **src/lib.rs:3065-3072** - Added `downloader_clone` for spawned download tasks in queue processor
3. **src/lib.rs:3142-3160** - Added post-processing spawn when pending_articles is empty in queue processor
4. **src/lib.rs:3592-3628** - Added post-processing spawn after DownloadComplete event in queue processor
5. **src/lib.rs:3907-3915** - Added `downloader` clone in `spawn_download_task()`
6. **src/lib.rs:3945-3960** - Added post-processing spawn when pending_articles is empty in spawn_download_task
7. **src/lib.rs:4177-4197** - Added post-processing spawn after DownloadComplete event in spawn_download_task
8. **src/lib.rs:645-658** - Updated `resume_download()` to call start_post_processing instead of TODO

All post-processing triggers spawn async tasks to avoid blocking the download completion path.

**Build Status**: ✓ Compiles successfully with no errors (cargo check passed)

