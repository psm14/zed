# Agent As Collaboration Partner (V1)

Status: Draft v0.1  
Owner: TBD  
Last updated: 2026-02-15

## Summary

This spec defines a UX-first path to make agents feel like active collaboration partners, not just tabs.

The plan is to use existing client-side collaboration primitives (agent follow, agent location, thread routing, notifications) to surface agent presence in collaboration surfaces before doing heavy collab server work.

## Problem

1. Agent activity currently feels private to the agent panel/tab workflow.
2. Collaborators can follow humans through collab UX, but agent presence is not surfaced similarly.
3. Users miss what the agent is doing unless they are already in the right thread.

## Goals

1. Surface agents as collaboration partners in existing collaboration UI.
2. Improve discoverability of active agent work and where it is happening.
3. Reuse existing follow/location behavior to minimize implementation risk.
4. Preserve current collab server constraints while improving UX now.

## Non-Goals (V1)

1. Full shared external-agent execution over collab.
2. Shared terminal execution in collaborative projects.
3. New heavy collab protocol/state systems for rich agent thread replication.

## Current State (Implementation Notes)

1. Agent follow primitives already exist through `CollaboratorId::Agent`.
2. Agent location updates already exist and drive follow behavior.
3. Agent thread tab dedupe/focus/routing is already strong.
4. External agents over collab are intentionally blocked today.
5. Terminal in collaborative projects is intentionally blocked today.
6. Collaborative text-thread transport exists, but remote open wiring in panel delegate is incomplete.

## Product Principles

1. Presence before parity: users should see agent activity even when full shared execution is unavailable.
2. Reuse familiar collab patterns: following an agent should feel like following a collaborator.
3. Progressive disclosure: compact, high-signal status by default; details on demand.
4. Honest constraints: blocked actions should explain why and what is supported.

## V1 UX Surfaces

1. Title bar collaborator area: add an agent presence chip.
2. Collab panel: add an `Agents` section with active sessions and status.
3. Agent thread tabs: show thread identity and status, not only provider label.
4. Keep existing notification routing behavior (focus editor tab if visible, else panel).

## Agent Status Model

1. Idle: thread exists with no active turn.
2. Reading: agent location updates while read-oriented tools/actions run.
3. Editing: agent location updates during edits.
4. Waiting for approval: tool authorization required.
5. Error: thread error state.
6. Done: turn ended successfully.

## Interaction Spec

1. Agent presence chip:
2. Single click toggles follow/unfollow.
3. Secondary action opens active thread.
4. Tooltip includes status and thread title.
5. Collab panel `Agents` row:
6. Shows agent icon, thread title, status, and optional current/last file.
7. Row click follows agent location.
8. Row action opens the thread.
9. Thread tabs:
10. Title should use thread title (fallback to current behavior when unavailable).
11. Status affordance (dot/spinner/icon) indicates current state.
12. Preserve existing tab open/focus/dedupe semantics.

## Technical Design (V1)

1. Introduce lightweight client-side `AgentPresence` projection from existing signals.
2. Inputs:
3. Project agent location updates.
4. ACP thread status/events and authorization-required events.
5. Workspace follow state for `CollaboratorId::Agent`.
6. UI integration targets:
7. `crates/title_bar/src/collab.rs` for title bar presence chip.
8. `crates/collab_ui/src/collab_panel.rs` for `Agents` section.
9. `crates/agent_ui/src/agent_panel.rs` for tab identity/status rendering.
10. `crates/agent_ui/src/acp/thread_view/active_thread.rs` for follow affordances.
11. No required collab DB/protocol changes for V1.

## V1.1 (Low-Risk Follow-Up)

1. Finish remote text-thread open path in agent panel delegate.
2. Reuse existing context RPC transport already present in collab.
3. Outcome: collaborators can open shared text-thread contexts through collaboration flows.

## Telemetry

1. `AgentPresenceChipShown`
2. `AgentPresenceChipClicked` with action `follow|unfollow|open_thread`
3. `CollabAgentsSectionOpened`
4. `CollabAgentRowClicked`
5. `BlockedFeatureShown` with reason `external_agent_collab|terminal_collab`
6. Correlate with existing `Follow Agent Selected`

## Success Metrics

1. Higher follow-agent usage per active session.
2. Lower time-to-open active thread after notification.
3. Increased interaction with agent collaboration surfaces.
4. No increase in notification-noise complaints.

## Rollout

1. Feature flag: `agent_partner_presence_v1`
2. Phase 1: title bar chip + thread tab identity/status.
3. Phase 2: collab panel `Agents` section.
4. Phase 3: V1.1 remote text-thread open path.

## Acceptance Criteria

1. Agent presence appears when there is an active agent thread.
2. Follow/unfollow from collaboration surfaces matches existing follow semantics.
3. Collab panel renders active agent entries with live status updates.
4. Thread tabs show thread identity and status affordance.
5. Blocked collab actions show clear and actionable messaging.
6. No regressions to existing thread tab dedupe/focus behavior.
7. Existing notification suppression behavior remains intact.

## Open Questions

1. Should presence show only generating threads, or the most recently active thread?
  - Align it with open agent sessions/tabs?
2. Should the collab panel show one agent row or multiple active thread rows?
  - There can be multiple agents running so try to account for that
3. Should waiting-for-approval states be pinned/high-priority in collab UI?
  - Doesn't need to be for V1
4. Should auto-follow on send be enabled in collaborative contexts by default?
  - Off-by-default