object DISCORD_SPEAKER [
  import_export_id -> "discord_speaker",
  import_export_hierarchy -> {"integrations"}
]
  name: "Generic Discord Speaker"
  parent: THING
  owner: ARCH_WIZARD
  readable: true

  property recent (owner: ARCH_WIZARD, flags: "r") = {};
  property recent_limit (owner: ARCH_WIZARD, flags: "rc") = 10;

  override description (owner: ARCH_WIZARD, flags: "rc") = "A device that carries what happens in this room to a Discord channel, and speaks what is said there.";
  override portable (owner: ARCH_WIZARD, flags: "rc") = false;

  method tell owner: ARCH_WIZARD
    "Relay what this room announces to Discord. Only the room's own announcements are heard.";
    const {event} = args;
    caller == this.location && valid(caller) || return;
    typeof(event) == TYPE_FLYWEIGHT && `event:validate() ! ANY => false' || return;
    event.actor == this && return;
    this in $discord_bridge.speakers || return;
    let content = "";
    try
      content = this:_render(event);
    except (ANY)
      return;
    endtry
    length(content) || return;
    this:_remember(content);
    $discord_bridge:relay(this, event.actor, content);
  endmethod

  method _render owner: ARCH_WIZARD
    "Render an event as plain text, as an ordinary listener in the room would read it.";
    caller == this || raise(E_PERM);
    const {event} = args;
    set_task_perms(this);
    return $document:render(event:resolve(this), 'text_plain)['body]:trim();
  endmethod

  method speak owner: ARCH_WIZARD
    "Say a channel message here. A linked author is named as their player; others are marked as from Discord.";
    caller == $discord_bridge || raise(E_PERM);
    const {author, display_name, text} = args;
    const room = this.location;
    valid(room) || return false;
    const name = valid(author) ? $sub:reference(author, 'label) | display_name + " (Discord)";
    const event = $event:mk_discord(this, $sub:reference(this, 'label), ": ", name, ": " + text):with_this(room);
    this:_remember((valid(author) ? author:name() | display_name + " (Discord)") + ": " + text);
    room:announce(event, {this});
    return true;
  endmethod

  method _remember owner: ARCH_WIZARD
    "Keep the latest lines for listen.";
    caller == this || raise(E_PERM);
    const {line} = args;
    let recent = {@this.recent, line};
    length(recent) > this.recent_limit && (recent = recent[$ - this.recent_limit + 1..$]);
    this.recent = recent;
  endmethod

  verb listen (any any any) owner: ARCH_WIZARD flags: "rxd"
    "Hear the latest lines that crossed this speaker, as 'listen radio' or 'listen to radio'.";
    this:_play_recent();
  endverb

  method _play_recent owner: ARCH_WIZARD
    "Tell the player the latest lines that crossed this speaker.";
    caller == this || raise(E_PERM);
    const lines = length(this.recent) ? this.recent | {"Only static so far."};
    const content = $format.block:mk("The latest from " + this:name() + ":", $format.list:mk(lines));
    player:inform_current($event:mk_info(player, content):with_presentation_hint('inset));
  endmethod
endobject
