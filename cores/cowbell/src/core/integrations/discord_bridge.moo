object DISCORD_BRIDGE [
  import_export_id -> "discord_bridge",
  import_export_hierarchy -> {"integrations"}
]
  name: "Discord Bridge"
  parent: ROOT
  owner: ARCH_WIZARD
  readable: true

  property api_base (owner: ARCH_WIZARD, flags: "") = "https://discord.com/api/v10";
  property batch_seconds (owner: ARCH_WIZARD, flags: "") = 5;
  property bot_token (owner: ARCH_WIZARD, flags: "") = "";
  property channel_id (owner: ARCH_WIZARD, flags: "") = "";
  property channel_members (owner: ARCH_WIZARD, flags: "") = {};
  property cursor (owner: ARCH_WIZARD, flags: "") = "";
  property enabled (owner: ARCH_WIZARD, flags: "") = false;
  property flush_schedule (owner: ARCH_WIZARD, flags: "") = 0;
  property inbound (owner: ARCH_WIZARD, flags: "") = {};
  property inbound_per_minute (owner: ARCH_WIZARD, flags: "") = 20;
  property max_length (owner: ARCH_WIZARD, flags: "") = 400;
  property outbox (owner: ARCH_WIZARD, flags: "") = {};
  property poll_schedule (owner: ARCH_WIZARD, flags: "") = 0;
  property poll_seconds (owner: ARCH_WIZARD, flags: "") = 10;
  property presence (owner: ARCH_WIZARD, flags: "") = [];
  property presence_quiet_seconds (owner: ARCH_WIZARD, flags: "") = 600;
  property speakers (owner: ARCH_WIZARD, flags: "") = {};
  property transport (owner: ARCH_WIZARD, flags: "") = #-1;
  property webhook_url (owner: ARCH_WIZARD, flags: "") = "";
  property world_name (owner: ARCH_WIZARD, flags: "") = "the MOO";

  override description (owner: ARCH_WIZARD, flags: "rc") = "Relays what Discord speakers hear, and who arrives and leaves, to a Discord channel through its webhook, and speaks the channel's messages through the speakers. Uses the curl worker: posts go to the channel webhook and the bot reads the channel by polling.";

  method configure owner: ARCH_WIZARD
    "Set the channel's webhook, the bot token and the channel id, then start polling and listening for presence.";
    caller_perms().wizard || raise(E_PERM);
    const {webhook_url, bot_token, channel_id, ?world_name = this.world_name} = args;
    typeof(webhook_url) == TYPE_STR && webhook_url:starts_with("https://") || raise(E_INVARG, "The webhook URL must be https.");
    typeof(bot_token) == TYPE_STR && length(bot_token) >= 32 || raise(E_INVARG, "The bot token looks too short.");
    typeof(channel_id) == TYPE_STR && pcre_match(channel_id, "^[0-9]{1,20}$") || raise(E_INVARG, "The channel id must be digits.");
    typeof(world_name) == TYPE_STR && length(world_name) || raise(E_INVARG);
    this.webhook_url = webhook_url;
    this.bot_token = bot_token;
    channel_id == this.channel_id || (this.cursor = "");
    this.channel_id = channel_id;
    this.world_name = world_name;
    this in $presence_listeners || ($presence_listeners = {@$presence_listeners, this});
    this.enabled = true;
    schedule_stop(this.poll_schedule);
    this.poll_schedule = schedule_every(this, "_poll", this.poll_seconds, {}, ["pass_elapsed" -> false, "jitter" -> 1]);
    return true;
  endmethod

  method disable owner: ARCH_WIZARD
    "Stop relaying in both directions; configuration is kept.";
    caller_perms().wizard || raise(E_PERM);
    this.enabled = false;
    schedule_stop(this.poll_schedule);
    this.poll_schedule = 0;
    $presence_listeners = setremove($presence_listeners, this);
    return true;
  endmethod

  method add_speaker owner: ARCH_WIZARD
    "Let a speaker relay what it hears and speak what the channel says.";
    caller_perms().wizard || raise(E_PERM);
    const {speaker} = args;
    valid(speaker) && isa(speaker, $discord_speaker) || raise(E_INVARG);
    this.speakers = setadd(this.speakers, speaker);
    return true;
  endmethod

  method remove_speaker owner: ARCH_WIZARD
    "Stop a speaker relaying or speaking.";
    caller_perms().wizard || raise(E_PERM);
    const {speaker} = args;
    this.speakers = setremove(this.speakers, speaker);
    return true;
  endmethod

  method relay owner: ARCH_WIZARD
    "Queue a line a registered speaker heard; a linked actor's name becomes a mention of their Discord account.";
    const {speaker, actor, text} = args;
    caller == speaker && speaker in this.speakers || raise(E_PERM);
    this.enabled || return false;
    const mention = this:_mention(actor);
    const name = length(mention) ? actor:name() | "";
    if (length(mention) && text:starts_with(name))
      const rest = text[length(name) + 1..$];
      this:_queue(rest, rest:starts_with(" ") ? mention + " " | mention);
    else
      this:_queue(text);
    endif
    return true;
  endmethod

  method presence_event owner: ARCH_WIZARD
    "Post a first connection, a new player or a last disconnection, once per quiet period.";
    caller == #0 || caller_perms().wizard || raise(E_PERM);
    const {user, kind} = args;
    this.enabled && valid(user) && is_player(user) || return false;
    kind in {'created, 'connected, 'disconnected} || raise(E_INVARG);
    const key = tostr(user, " ", kind);
    const last = `this.presence[key] ! E_RANGE => 0';
    kind != 'created && time() - last < this.presence_quiet_seconds && return false;
    this.presence[key] = time();
    const name = user:name();
    const text = kind == 'created ? "A new guest, " + name + ", has arrived for the first time." | kind == 'connected ? name + " has arrived at " + this.world_name + "." | name + " has left.";
    this:_queue(text);
    return true;
  endmethod

  method _poll owner: ARCH_WIZARD
    "Fetch the channel's new messages and speak them; the first poll only finds where the channel is.";
    caller_perms().wizard || raise(E_PERM);
    this.enabled && length(this.channel_id) || return 0;
    const first = !length(this.cursor);
    const query = first ? "?limit=1" | "?limit=50&after=" + this.cursor;
    let response = {};
    try
      response = this:_request("GET", this.api_base + "/channels/" + this.channel_id + "/messages" + query, "", {{"Authorization", "Bot " + this.bot_token}});
    except error (ANY)
      server_log(tostr("Discord bridge poll failed: ", toliteral(error[1])));
      return 0;
    endtry
    const status = `response[1] ! ANY => 0';
    if (status in {401, 403})
      server_log(tostr("Discord bridge stopped polling: the channel refused the bot token with HTTP ", status));
      schedule_stop(this.poll_schedule);
      this.poll_schedule = 0;
      return 0;
    endif
    "A rate limit or an outage is retried on the next poll.";
    status == 200 || return 0;
    const messages = `parse_json(response[3]) ! ANY => {}';
    typeof(messages) == TYPE_LIST || return 0;
    "An empty channel has no history to skip, so everything after it is new.";
    first && !length(messages) && (this.cursor = "0");
    const ids = { `message["id"] ! ANY => "0"' for message in (messages) };
    let spoken = 0;
    for message in (sort(messages, ids, true))
      const id = `message["id"] ! ANY => ""';
      if (typeof(id) == TYPE_STR && pcre_match(id, "^[0-9]{1,20}$"))
        this.cursor = id;
        first || (spoken = spoken + this:_receive(message));
      endif
    endfor
    return spoken;
  endmethod

  method _receive owner: ARCH_WIZARD
    "Speak one channel message in every speaker's room; bots, webhooks and system messages are skipped.";
    caller == this || raise(E_PERM);
    const {message} = args;
    typeof(message) == TYPE_MAP || return 0;
    "Webhook posts include the bridge's own, so skipping them keeps the bridge from echoing itself.";
    maphaskey(message, "webhook_id") && return 0;
    `message["type"] ! E_RANGE => -1' in {0, 19} || return 0;
    const author = `message["author"] ! E_RANGE => []';
    typeof(author) == TYPE_MAP || return 0;
    `author["bot"] ! E_RANGE => false' == true && return 0;
    const discord_id = `author["id"] ! E_RANGE => ""';
    const global_name = `author["global_name"] ! E_RANGE => ""';
    const display_name = this:_clean(typeof(global_name) == TYPE_STR && length(global_name) ? global_name | `author["username"] ! E_RANGE => ""', 64);
    const text = this:_clean(`message["content"] ! E_RANGE => ""', this.max_length);
    typeof(discord_id) == TYPE_STR && length(display_name) && length(text) || return 0;
    this:_admit() || return 0;
    "Whoever posts here is a member of the channel's server, so a mention of them will resolve.";
    this.channel_members = setadd(this.channel_members, discord_id);
    const linked = $login:find_by_oauth2("discord", discord_id);
    const linked_player = valid(linked) && is_player(linked) ? linked | #-1;
    for speaker in (this.speakers)
      valid(speaker) && `speaker:speak(linked_player, display_name, text) ! ANY';
    endfor
    return 1;
  endmethod

  method _mention owner: ARCH_WIZARD
    "A Discord mention for an actor whose linked account has posted in the channel, or the empty string.";
    caller == this || raise(E_PERM);
    const {actor} = args;
    typeof(actor) == TYPE_OBJ && valid(actor) && is_player(actor) || return "";
    for identity in (`actor.oauth2_identities ! ANY => {}')
      if (typeof(identity) == TYPE_LIST && length(identity) == 2 && identity[1] == "discord" && identity[2] in this.channel_members)
        return "<@" + identity[2] + ">";
      endif
    endfor
    return "";
  endmethod

  method _request owner: ARCH_WIZARD
    "Make one HTTP request, or ask the test transport when one is set.";
    caller == this || raise(E_PERM);
    const {method, url, body, headers} = args;
    valid(this.transport) && return this.transport:request(method, url, body, headers);
    return $http:request(method, url, body, headers, ["max_bytes" -> 262144, "timeout_seconds" -> 10.0]);
  endmethod

  method _admit owner: ARCH_WIZARD
    "Count an inbound message against the per-minute limit.";
    caller == this || raise(E_PERM);
    const now = time();
    const recent = { stamp for stamp in (this.inbound) if now - stamp < 60 };
    length(recent) < this.inbound_per_minute || return false;
    this.inbound = {@recent, now};
    return true;
  endmethod

  method _clean owner: ARCH_WIZARD
    "Drop control characters and cut to a length, marking the cut.";
    caller == this || raise(E_PERM);
    const {value, limit} = args;
    typeof(value) == TYPE_STR || return "";
    const text = pcre_replace(value, "s/[\\x00-\\x1f\\x7f]+/ /g"):trim();
    return length(text) > limit ? text[1..limit - 1] + "…" | text;
  endmethod

  method _escape owner: ARCH_WIZARD
    "Escape Discord markdown so hotel text shows as written.";
    caller == this || raise(E_PERM);
    const {text} = args;
    return pcre_replace(text, "s/([\\\\*_~`|>#\\[\\]()-])/\\$1/g");
  endmethod

  method _queue owner: ARCH_WIZARD
    "Add a line to the outbox, after an optional mention, and make sure a flush is coming.";
    caller == this || raise(E_PERM);
    const {text, ?mention = ""} = args;
    const line = mention + this:_escape(this:_clean(text, this.max_length));
    length(line) || return;
    this.outbox = {@this.outbox, line};
    schedule_valid(this.flush_schedule) && return;
    this.flush_schedule = schedule_at(this, "_flush", time() + this.batch_seconds, {}, ["adaptive" -> false, "pass_elapsed" -> false]);
  endmethod

  method _flush owner: ARCH_WIZARD
    "Post the outbox under the world's name, packing lines into as few posts as Discord allows.";
    caller_perms().wizard || raise(E_PERM);
    const lines = this.outbox;
    this.outbox = {};
    this.flush_schedule = 0;
    let posts = {};
    for line in (lines)
      if (length(posts) && length(posts[$]) + 1 + length(line) <= 1900)
        posts[$] = posts[$] + "\n" + line;
      else
        posts = {@posts, line};
      endif
    endfor
    for post in (posts)
      this:_post(post);
    endfor
    return length(posts);
  endmethod

  method _post owner: ARCH_WIZARD
    "Send one post to the channel webhook, retrying once after a rate limit.";
    caller == this || raise(E_PERM);
    const {content} = args;
    const username = this.world_name[1..min(80, length(this.world_name))];
    const body = generate_json(["username" -> username, "content" -> content, "allowed_mentions" -> ["parse" -> {}]]);
    for attempt in [1..2]
      let response = {};
      try
        response = this:_request("POST", this.webhook_url, body, {{"Content-Type", "application/json"}});
      except error (ANY)
        server_log(tostr("Discord bridge post failed: ", toliteral(error[1])));
        return false;
      endtry
      const status = `response[1] ! ANY => 0';
      status >= 200 && status < 300 && return true;
      if (status == 429 && attempt == 1)
        const wait = `parse_json(response[3])["retry_after"] ! ANY => 1';
        suspend(min(10, max(1, toint(wait))));
        continue;
      endif
      server_log(tostr("Discord bridge post refused with HTTP ", status));
      return false;
    endfor
    return false;
  endmethod
endobject
