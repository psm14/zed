# Agent As Collaboration Partner (V2: Collab Server State + Thread Sharing)

Status: Draft v0.1  
Owner: TBD  
Last updated: 2026-02-19

## Summary

This spec defines the first collab-server-backed protocol for sharing agent presence and agent thread snapshots with call participants.

V1 delivered local UI presence in collaboration surfaces. V2 adds wire-level transport so collaborators can see remote agent activity and open/sync agent threads through shared projects.

## Problem

1. Agent presence in collab UI is currently local to the current window/session.
2. Remote collaborators cannot discover remote agent thread state from collab transport.
3. Sharing/importing agent threads currently uses a global link flow (`ShareAgentThread` / `GetSharedAgentThread`) instead of room/project-scoped collab semantics.
4. `open_remote_text_thread` is currently a stub in the concrete delegate path, which blocks full follow/open parity for shared thread-like surfaces.

## Goals

1. Broadcast per-participant agent presence to room members in near real time.
2. Allow collaborators in a shared project to open/sync agent thread snapshots through collab RPC.
3. Keep V2 low-risk by reusing existing collab primitives:
   1. room membership and participant identity
   2. project host-forwarding request paths
   3. project host broadcast paths
4. Preserve current constraints around external-agent execution and shared terminal execution in collab.

## Non-Goals (V2)

1. Shared execution control of remote agent turns.
2. Shared terminal execution in collaborative projects.
3. Collaborative approval actions for tool authorization.
4. Replacing public link sharing (`ShareAgentThread`) for out-of-room use cases.

## Current Baseline

1. Room and project collaboration transport already exists (`RoomUpdated`, `ShareProject`, `JoinProject`, forward/broadcast project message handlers).
2. Text-thread collab transport exists (`AdvertiseContexts`, `OpenContext`, `SynchronizeContexts`, `UpdateContext`) and is an implementation model.
3. Agent thread link sharing already exists (`ShareAgentThread`, `GetSharedAgentThread`) and can provide snapshot format reuse.
4. Agent location/follow is local and session-targeted (`CollaboratorId::Agent` + selected `agent_session_id`), not room-shared agent identity.

## Proposed Architecture

### 1) Room-Scoped Agent Presence Stream

Add a lightweight room-level presence channel:

1. Host/client publishes local agent session presence for active threads in the window.
2. Collab server validates room membership and project scoping, stores the latest payload per `(room_id, peer_id)`, and broadcasts to room participants.
3. Recipients merge into room state and render remote agent rows under the corresponding shared project.

Key property: this is ephemeral server state (no DB migration in V2).

### 2) Project-Scoped Agent Thread Snapshot Access

Add project-scoped RPC to open/sync agent thread snapshots:

1. Host advertises available agent thread metadata for a shared project.
2. Guest requests a specific thread snapshot by `session_id` through project forwarding.
3. Host responds with serialized snapshot bytes and revision metadata.
4. Guest stores/imports and opens thread locally; manual or periodic sync uses revision checks.
5. Host continues to push revision metadata updates (`AdvertiseAgentThreads`), while snapshot payload transfer remains request/response (`OpenAgentThread` and `SynchronizeAgentThreads`).

Key property: this mirrors text-thread collab transport patterns while keeping V2 low-risk via metadata push + snapshot pull.

## Proposed Proto Additions

### `call.proto` additions (room presence)

```proto
enum AgentSessionStatus {
    AGENT_SESSION_STATUS_UNSPECIFIED = 0;
    AGENT_SESSION_STATUS_IDLE = 1;
    AGENT_SESSION_STATUS_READING = 2;
    AGENT_SESSION_STATUS_EDITING = 3;
    AGENT_SESSION_STATUS_GENERATING = 4;
    AGENT_SESSION_STATUS_WAITING_FOR_APPROVAL = 5;
    AGENT_SESSION_STATUS_ERROR = 6;
}
// `DONE` is intentionally omitted in V2; completed turns collapse to `IDLE`.

message AgentSessionPresence {
    string session_id = 1;              // acp::SessionId UUID string
    optional uint64 project_id = 2;     // only shared project ids should be sent
    string title = 3;
    AgentSessionStatus status = 4;
    optional string relative_file = 5;  // project-relative path only
    uint64 updated_at_unix_ms = 6;
}

message UpdateAgentPresence {
    uint64 room_id = 1;
    repeated AgentSessionPresence sessions = 2; // empty => clear
}

message AgentPresenceUpdated {
    uint64 room_id = 1;
    PeerId peer_id = 2;
    repeated AgentSessionPresence sessions = 3;
    uint64 generation = 4; // monotonic per (room_id, peer_id)
}
```

### `ai.proto` additions (project thread metadata + snapshot open/sync)

```proto
message AgentThreadMetadata {
    string session_id = 1;
    string title = 2;
    uint64 updated_at_unix_ms = 3;
    uint64 revision = 4;
}

message AdvertiseAgentThreads {
    uint64 project_id = 1;
    repeated AgentThreadMetadata threads = 2;
}

message OpenAgentThread {
    uint64 project_id = 1;
    string session_id = 2;
}

message OpenAgentThreadResponse {
    string session_id = 1;
    string title = 2;
    bytes thread_data = 3;      // SharedThread-compatible payload
    uint64 revision = 4;
    uint64 host_user_id = 5;
    string host_username = 6;
}

message AgentThreadVersion {
    string session_id = 1;
    uint64 revision = 2;
}

message SynchronizeAgentThreads {
    uint64 project_id = 1;
    repeated AgentThreadVersion threads = 2;
}

message SynchronizeAgentThreadsResponse {
    repeated AgentThreadVersion threads = 1; // server returns host versions
}
```

### `zed.proto` oneof additions

Add message variants for:

1. `UpdateAgentPresence`
2. `AgentPresenceUpdated`
3. `AdvertiseAgentThreads`
4. `OpenAgentThread`
5. `OpenAgentThreadResponse`
6. `SynchronizeAgentThreads`
7. `SynchronizeAgentThreadsResponse`

Wire IDs should be allocated at implementation time from next available slots to avoid merge conflicts.

## RPC / Server Design

## `collab/src/rpc.rs` registration changes

Add:

1. `add_message_handler(update_agent_presence)`
2. `add_message_handler(broadcast_project_message_from_host::<proto::AdvertiseAgentThreads>)`
3. `add_request_handler(forward_mutating_project_request::<proto::OpenAgentThread>)`
4. `add_request_handler(forward_mutating_project_request::<proto::SynchronizeAgentThreads>)`

## New handler: `update_agent_presence`

Server behavior:

1. Validate sender is an active participant in `room_id`.
2. Validate each presence entry:
   1. `session_id` format is valid UUID string.
   2. if `project_id` exists, ensure sender currently shares that project in the room.
   3. `relative_file` is relative and size-limited.
3. Store latest payload in ephemeral memory keyed by `(room_id, peer_id)` with incremented generation.
4. Broadcast `AgentPresenceUpdated` to room participants.

Lifecycle cleanup:

1. On leave room / disconnect: clear all presence for sender in that room and broadcast empty update.
2. On unshare project: remove entries referencing that `project_id` and broadcast trimmed payload.

## Thread RPC behavior

`OpenAgentThread` and `SynchronizeAgentThreads` use existing project host forwarding.

1. Guest sends request on project entity channel.
2. Collab server forwards to host connection for that project.
3. Host-side agent store returns metadata/snapshot.
4. Host broadcasts revision metadata changes via `AdvertiseAgentThreads`; host does not push `thread_data` snapshots unsolicited in V2.

No collab DB persistence is required in V2 for agent thread snapshots.

## Client Design

### Room presence ingestion

In `call` room state:

1. Subscribe to `AgentPresenceUpdated`.
2. Maintain `agent_presences_by_peer_id: HashMap<PeerId, AgentPresenceState>`.
3. Emit room event on update for collab UI refresh.

### Local presence publishing

In `agent_ui`:

1. Observe `AgentPanel::collaboration_presences(...)` and map to proto status/metadata.
2. Publish debounced `UpdateAgentPresence` while in active room.
3. Publish empty payload on panel teardown, room leave, or no active sessions.

### Collab panel rendering

In `collab_ui`:

1. Prefer room-shared presences for both local and remote participants.
2. Keep project-anchored rendering under `ParticipantProject` rows.
3. For sessions without valid `project_id`, render in standalone `Agents` section.
4. Fallback when no room:
   1. preserve current local-only `Agents` behavior.

### Follow semantics alignment

1. Local rows already toggle follow/unfollow for the matching `session_id`.
2. Selecting a different session switches the active follow target instead of requiring an explicit global unfollow.
3. V2 remote presence should preserve this per-session follow model as remote location transport is added.

### Open remote agent thread

Row click behavior for remote rows:

1. Ensure user has joined the project (`join_in_room_project` path).
2. Request `OpenAgentThread(project_id, session_id)`.
3. Save into local `ThreadStore` with imported metadata using the same imported-thread path as existing link sharing for upstream compatibility.
4. Open/focus in Agent panel tab.

`SynchronizeAgentThreads` can power refresh/manual sync actions.

## Security / Privacy / Limits

1. Presence visibility is room-scoped only.
2. Thread snapshot visibility is project-collaborator scoped via existing project permissions.
3. Paths must be project-relative (never absolute).
4. Recommended server limits:
   1. max 32 sessions per participant presence update
   2. max title length 256 bytes
   3. max relative_file length 512 bytes
   4. max `thread_data` payload 2 MiB per snapshot
5. Rate-limit `UpdateAgentPresence` per connection to avoid update floods.

## Rollout

1. Feature flag: `agent_partner_collab_server_v2`
2. Phase 1:
   1. room presence protocol + collab panel rendering of remote agent rows
3. Phase 2:
   1. `AdvertiseAgentThreads` + `OpenAgentThread` + delegate wiring for remote open
4. Phase 3:
   1. `SynchronizeAgentThreads` for revision-based sync/refresh

## Acceptance Criteria

1. Room participants see remote agent sessions with live status changes.
2. Agent rows appear under shared project rows when `project_id` matches.
3. Clicking a remote agent row can open/import its thread when project access is granted.
4. Presence state is removed promptly when participant leaves room or unshares project.
5. No regression to existing session-targeted follow semantics, tab dedupe/focus, or notification routing.
6. Existing public link thread sharing remains functional.

## Testing Plan

1. Collab server unit/integration tests:
   1. `UpdateAgentPresence` requires room membership.
   2. project-scoped entries are rejected when sender does not share project.
   3. leave/unshare clears and rebroadcasts presence.
2. Room/client tests:
   1. presence updates merge and replace by generation order.
   2. selecting a different session updates follow target without breaking active follow state.
3. UI tests:
   1. collab panel places remote rows under participant projects.
   2. fallback standalone `Agents` section still works without active room.
4. Thread open/sync tests:
   1. guest can open host thread snapshot after joining project.
   2. unauthorized guest cannot open thread.

## Resolved Decisions (2026-02-19)

1. V2 uses host-pushed metadata updates and pull-based snapshot payload transfer:
   1. Host pushes thread revision/title metadata with `AdvertiseAgentThreads`.
   2. Guests fetch snapshot bytes via `OpenAgentThread` and refresh via `SynchronizeAgentThreads`.
   3. Host does not push `thread_data` payloads unsolicited in V2.
2. Collab-imported threads keep the same visual identity as existing link-imported threads for compatibility:
   1. A "link-imported thread" is one imported through `ShareAgentThread` / `GetSharedAgentThread` (for example via `zed://agent/shared/<session_id>`).
   2. V2 collab import should reuse the same imported-thread affordances and metadata path rather than introducing a distinct visual style.
3. `DONE` is removed from the V2 presence protocol model and collapses to `IDLE`.
