# WebSocket support proposal

Work in progress for NIL-36. This document records the contribution's scope;
it does not describe an available host API. See the host API's **Coming later**
section for the existing framework intent.

Niles will own WebSocket connections for sandboxed add-ons, reconnect them,
and deliver messages and connection status through serialized callbacks.
The implementation will use explicit household-approved permissions and
bounded connections, messages and queues. It will make connection loss and
queue overflow visible so add-ons can re-establish their application state.
Existing interface versions must continue to work.

Verification will include an actual Wasm add-on communicating with a local
synthetic WebSocket server, plus permission, lifecycle, resource-limit and
compatibility tests. Implementation and verification are still in progress.

Autodarts provides a concrete use case for local device events. Its scoring
logic, speech policy, satellite targeting and deployment are outside this
contribution.
