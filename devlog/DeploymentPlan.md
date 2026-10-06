# Deployment Plan — from desktop app to a hosted service on AWS

Rev 1 · 2026-09-28 · **Strategy only. Nothing here is scheduled for implementation.**

Where the analyzer should go to become a hosted beta for the VGC community. It records the
decisions made so far and the questions still open. `Decided:` marks things agreed with the
user; everything else is a proposal. **[unverified]** = not checked in-session.

---

## 1. Where it stands

- One `compose.yaml` stack: `web` (nginx serving the React build, proxying `/api`),
  `api` (Rust axum, runs transcription in-process), `postgres` (17-alpine).
- **Uploads stream through the API** to a local volume (`server/src/main.rs`, `write_body`).
- **One job at a time, in memory:** a `Semaphore(1)` in the API process. Live progress is
  held in memory; a restart marks in-flight runs as failed.
- **The preview keeps audio** (`analyzer/src/decode.rs`, `Extras`): H.264, 1280 wide, CRF 26.
- **Migrations are safe for several replicas:** `pokedex/src/migrate.rs` takes an advisory lock.
- **Inputs are big:** ~1.1 GB per 10 minutes (HEVC, 15 Mbps). `raw_recordings/` is 89 GB
  for 103 videos.
- **Performance:** decode-bound; see [AnalyzerPerformance.md](AnalyzerPerformance.md). The
  CPU cost per video-minute has not been measured yet.

## 2. Decisions so far

- **Decided: AWS, on EKS.** Accepted knowingly: EKS has a fixed monthly floor with zero
  users. [unverified, list prices] control plane ~$73, NAT gateway ~$32, load balancer ~$16,
  before any compute. ECS Fargate would be simpler to run, but Kubernetes is the choice.
- **Decided: don't keep originals.** Keep only a redacted, silent 720p H.264 preview.
  Delete the original from S3 after a successful transcription, with a grace period of a
  few days so failed runs can be retried.

## 3. Video storage

Kept per 10-minute video, by option:

| Option | Size | Privacy exposure |
|---|---|---|
| Original | ~1.1 GB | unredacted notifications, audio |
| **Redacted preview only (chosen)** | ~100–200 MB [unverified] | low |
| Transcript only | KB | none, but no player |

Proposal:
- **Browser → S3 directly** with presigned multipart upload URLs, not through the API.
  Relaying gigabytes through a pod breaks on load balancer timeouts and restarts, and costs
  API capacity.
- **Workers** download the object to node-local disk and run ffmpeg as today. `decode.rs`
  takes a path, so the core barely changes.
- **Playback** through CloudFront with signed URLs, issued only after a permission check.
  Egress is billed per GB [unverified ~$0.09/GB from S3], which is another reason for small
  previews.
- **S3 lifecycle rules:** expire originals after the grace period, and abort incomplete
  multipart uploads.
- **Quotas from day one:** maximum file size and videos per user. Public 1 GB uploads are
  the biggest cost risk.
- **Local parity:** a storage abstraction (disk or S3), with MinIO in compose.

## 4. Privacy: notification redaction

Only the preview is ever shown, and the analyzer reads only the message-line crop. So
redaction belongs in the preview branch of the existing ffmpeg filter graph.

1. **v1: always blur the band where iOS banners appear.** Deterministic, cheap, can't miss.
   Cost: that strip of game UI is always blurred. Needs the banner's position measured on
   real recordings.
2. **v2: detect banners and blur only those frames.** Similar to the segmentation work the
   analyzer already does. Needs a labelled set of frames with and without banners.

Also:
- **Drop audio from the preview** (`-an`). It can carry microphone audio, voices and
  notification sounds.
- **Tell users to turn on Do Not Disturb** before recording, on the upload page.
- Deleting originals (§2) is itself a privacy measure: the unredacted file exists only until
  the job finishes and the grace period ends.
- Browser-side decode (§8) would mean the raw file never leaves the user's machine at all.

## 5. Postgres

Proposal: **RDS**, not self-hosted in the cluster.
- Battle data is irreplaceable (see [SchemaIteration.md](SchemaIteration.md) §2). RDS gives
  automated backups and point-in-time recovery [unverified ~$15–30/month for a small
  Graviton instance].
- Self-hosting means running an operator (e.g. CloudNativePG), backing up to S3 ourselves,
  and dealing with EBS volumes pinned to one availability zone.

## 6. Authentication

Proposal:
- **Use an OIDC provider; don't build login.** The API validates JWTs in an axum
  middleware and maps the token's `sub` to a row in the trainer table (the "trainer table
  as the user table" TODO).
- **Keep an auth stub for local compose runs.** **Done 2026-10-06**
  (`specs/001-trainer-table`):
  - **What exists:** the `trainer` table, with an `auth_subject` column waiting for the
    token's `sub`.
  - **Where login plugs in:** a `Trainer` extractor in `server/src/auth.rs`, which the OIDC
    resolver will extend.
  - **The stub:** `DEV_AUTH=1`, which picks the trainer by cookie.
  - **Deploy rule:** the deployed API must not set `DEV_AUTH`. Without it, battle routes
    answer 401 until a real resolver is added.

Open: **which provider.** It depends on which "Sign in with …" buttons users should see.
- **Cognito:** AWS-native, cheap. Google, Apple and email are easy. Discord is awkward,
  because Discord's OAuth is not full OIDC and needs a workaround [unverified].
- **Auth0 / Clerk:** Discord built in, more expensive past their free tiers [unverified].
- The VGC community organises on Discord, so Discord login may be what users expect. If
  Google/Apple/email is enough, Cognito is fine.

## 7. Permissions

Proposal, deliberately small:
- Every video, battle and transcript has an **owner**; every query filters by owner.
- Roles: `user` and `admin` (pokedex imports, moderation).
- Pokedex data stays publicly readable.
- Later: unlisted share links for a battle.
- Optional safety net: Postgres row-level security.

## 8. Compute: jobs and workers

Proposal:
- **Split the API from the workers.** Replace the in-process `Semaphore(1)` with a job
  queue in Postgres (`SELECT … FOR UPDATE SKIP LOCKED`; no SQS needed). Workers run as their
  own Deployment.
- **Progress and the live snapshot** move out of API memory: progress to the DB, the
  snapshot to S3 or the DB.
- **Scaling:** workers on spot capacity, scaled by queue depth (KEDA).
- **Size the workers from a measurement:** CPU-seconds per video-minute
  (AnalyzerPerformance.md §5), then price CPU vs GPU from that.
- **Later levers** (AnalyzerPerformance.md §4): keyframe-chunked parallel decode, NVDEC,
  and browser-side decode. Browser decode would remove the upload wait (~7 min for 1.1 GB on
  a 20 Mbps uplink), most compute cost, and the privacy exposure of raw uploads. Doing the
  `FrameSource` trait refactor early keeps that path open.

## 9. Infrastructure outline

Proposal: Terraform or OpenTofu for:
- VPC, EKS, RDS, S3, CloudFront, ECR
- AWS Load Balancer Controller, ACM for TLS
- EKS Pod Identity for S3 access, External Secrets for credentials
- Frontend: keep the nginx container, or serve the static build from S3 + CloudFront
- CI that builds and pushes images
- Monitoring and **cost alarms** before any public beta

## 10. Rough phases

0. **Decide** the open questions below.
1. **Make the app cloud-shaped, locally:** storage abstraction + MinIO, API/worker split,
   redaction + silent preview, owner columns + auth stub. All testable in compose.
2. **Infrastructure** (§9).
3. **Closed beta:** real auth, quotas, monitoring.
4. **Optimise:** browser decode, chunked decode, GPU if the numbers justify it.

## 11. Open questions

- Auth provider: is Discord login needed? (§6)
- Grace period before originals are deleted: how many days?
- Preview resolution and bitrate: is 720p readable enough for review?
- Banner redaction: fixed band first, or go straight to detection?
- Frontend hosting: nginx pod or S3 + CloudFront?
- Budget ceiling for the beta, which bounds node sizes and quotas.
