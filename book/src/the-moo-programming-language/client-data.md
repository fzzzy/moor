# Structured Client Data

Besides narrative output and commands, a connection can exchange structured data with the core:
values such as a room's state, a map position, or a client's capabilities. They travel as data
events. A data event is a namespace and a kind, both symbols, and a payload, which is any MOO value.

- The core sends data with `emit_data()`.
- Clients send data to the handler's `do_client_data` verb.

Each host carries data in its own way. The telnet host uses GMCP and MSDP. The web host uses the
WebSocket and an optional WebRTC data channel.

## Sending data: `emit_data()`

```
int emit_data(obj|list recipients, symbol namespace, symbol kind, any payload [, map options])
```

`emit_data()` sends the event to each connection of each recipient. The caller must control every
recipient, or be a wizard. If any recipient is refused, nothing is sent. Like `notify()`, the events
are delivered when the task commits, so an aborted task sends nothing.

```moo
emit_data(player, 'state, 'room_snapshot, ["title" -> "The Lobby", "exits" -> {"north"}]);
emit_data(players_here, 'game, 'positions, positions, ['ephemeral -> true]);
```

The one option is `'ephemeral`. An unknown option raises `E_INVARG`, and a value that is not a
boolean raises `E_TYPE`.

### Durable and ephemeral events

By default an event is durable: like narrative output, it is written to the recipient's event log
and delivered reliably and in order. A web client that reconnects receives any durable events it
missed.

With `['ephemeral -> true]`, the event is for state that the next event replaces, such as positions
or a status panel:

- It is never written to the event log.
- It is delivered best-effort. It is not queued for acknowledgement, so a client that is
  disconnected, or that loses the event in transit, does not receive it later.
- It is latest-wins. A newer ephemeral event with the same namespace and kind replaces an older one
  for the same recipient that is not yet delivered. This applies within one task and while a
  client's connection is congested.
- On the web host, it takes the client's WebRTC data channel when one is open, and otherwise the
  WebSocket.
- It is not ordered with respect to durable events or narrative output.

Use durable events for anything the client must not miss, and ephemeral events for anything it can
skip because a newer value will arrive.

### What each host does with them

- **Web host.** Durable events, and ephemeral events without an open data channel, go to the
  WebSocket as client events. Ephemeral events on an open channel use the channel's format (see
  [The data channel](#the-data-channel)). SSE clients receive ephemeral events as `ephemeral`
  events, with no id.
- **Telnet host.** Events in the `'gmcp` namespace are sent as GMCP messages named by the kind, if
  the client has enabled GMCP and declared support for the package. Events in the `'msdp` namespace
  are sent as MSDP variables named by the kind, if the client has enabled MSDP. Other events are
  dropped.

## Receiving data: `do_client_data`

When a client sends data, the host calls a verb on its listener's handler object, normally `#0`:

```
handler:do_client_data(connection, namespace, kind, payload)
```

- `connection` is the connection object.
- `namespace` is a symbol, chosen by the host, not the client.
- `kind` is a string. Clients choose the kind, and symbols are never freed, so the server does not
  turn client-supplied names into symbols.

The task runs as the logged-in player, or, before login, as the connection object with the system
object's authority. The host neither waits for it nor reports its result to the client, and a
missing verb is not an error.

In Cowbell, `#0:do_client_data` hands each namespace to the installed package of the same name, as
`package:client_data(kind, payload)`. A package that receives client data defines `client_data` on
its package object; data for a namespace no package claims is ignored.

| Namespace | Sent by                                                        | `kind`                       |
| --------- | -------------------------------------------------------------- | ---------------------------- |
| `'gmcp`   | telnet GMCP messages                                           | the package and message name |
| `'msdp`   | telnet MSDP variables                                          | the variable name            |
| `'telnet` | telnet options and subnegotiations the host does not implement | `"negotiate"` or `"subneg"`  |
| `'web`    | web clients (see [From web clients](#from-web-clients))        | chosen by the client         |
| `'client` | either host, when the connection's capabilities change         | `"attributes"`               |

### Connection attributes

When a connection gains or loses a capability, the host sets a connection attribute and then sends a
`'client` `"attributes"` event. The payload maps each changed attribute to its new value, or to
`#-1` when the attribute was removed.

- The telnet host reports the protocols the client negotiates, such as `'gmcp`, `'msdp` and
  `'mccp2`, as `true` when enabled and `false` when disabled.
- The web host reports `'realtime -> true` when the client's WebRTC data channel opens, and
  `'realtime -> #-1` when it closes.

Read the current value with `connection_option(connection, "realtime")`, which raises `E_INVARG`
while the attribute is absent.

```moo
verb "do_client_data" (this none this) owner: ARCH_WIZARD flags: "rxd"
  {connection, namespace, kind, payload} = args;
  if (namespace == 'client && kind == "attributes")
    "Remember whether this connection can take fast updates.";
    return;
  elseif (namespace == 'web && kind == "move")
    "Handle the client's request.";
  endif
endverb
```

### From web clients

A web client sends data in either of two ways. Both reach `do_client_data` with the namespace
`'web`.

- A binary WebSocket message: the byte `0x05`, then the message.
- A message on its WebRTC data channel, in the format the channel chose (see
  [The data channel](#the-data-channel)).

A message is either JSON or FlatBuffers:

- **JSON**: an object `{"kind": "move", "payload": [3, 4]}`. A missing payload is `null`, and JSON
  booleans become `1` and `0`.
- **FlatBuffers**: a `ClientDataMessage` table from the mooR schema, with a `kind` string and a
  `payload` value.

A WebSocket message whose body starts with `{` is JSON; otherwise it is FlatBuffers. Messages over
16 KiB are rejected.

The web host limits each connection to a number of messages per second, 50 by default, counting the
WebSocket and the data channel together. Messages over the limit are dropped. Set the limit with
`client_data_rate`, under `services.web` for the combined `moor` server, or with
`--client-data-rate` for `moor-web-host`; `0` means unlimited.

## The data channel

A web client can open a WebRTC data channel beside its WebSocket. This needs WebRTC enabled on the
web host (see [Deployment](../web-client/deployment.md#webrtc-data-channels)).

The client signals over the WebSocket. A signaling message is a binary message: the byte `0x03`,
then JSON. The client sends `{"type": "offer", "sdp": "..."}` and may send
`{"type": "ice", "candidate": "..."}` candidates. The server replies with
`{"type": "answer", "sdp": "..."}`, which lists all of its own candidates. A new offer replaces the
connection's earlier data channel.

The client creates the channel. It chooses the channel's ordering and reliability; unordered and
unreliable gives the lowest latency. The channel's sub-protocol chooses its format, which applies in
both directions:

| Sub-protocol       | Format      | Events from the server                                |
| ------------------ | ----------- | ----------------------------------------------------- |
| `moor.fb`, or none | FlatBuffers | the same client event bytes as on the WebSocket       |
| `moor.json`        | JSON        | `{"namespace": "...", "kind": "...", "payload": ...}` |

The server does not use a channel with any other sub-protocol. On a JSON channel, an event whose
payload has no JSON form goes to the WebSocket instead.
