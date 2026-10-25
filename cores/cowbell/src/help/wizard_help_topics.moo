object WIZARD_HELP_TOPICS [
  import_export_id -> "wizard_help_topics",
  import_export_hierarchy -> {"help"}
]
  name: "Wizard Help Topics"
  parent: HELP_SOURCE
  location: PROTOTYPE_BOX
  owner: ARCH_WIZARD
  readable: true

  property topic_wizard_announce (owner: ARCH_WIZARD, flags: "rc") = <HELP, .name = "@announce", .content = "Usage: `@announce <message>`\n\nBroadcasts a message to all connected players.", .summary = "Broadcast announcement", .aliases = {"announce", "broadcast"}, .category = 'administration, .content_kind = 'djot_blocks, .see_also = {"@shutdown"}>;
  property topic_wizard_audit_names (owner: ARCH_WIZARD, flags: "rc") = <HELP, .name = "@audit-names", .content = "Usage: `@audit-names [--players | OBJECT]`\n\nReports suspect stored names and aliases on all objects by default. Use --players for accounts only, or an object reference for one object. The report includes object, owner, and location IDs, the affected field, an escaped value, and reasons.\n\nFindings include empty names, names without a visible base character, directional controls, control or format characters, edge whitespace, and malformed aliases. Formatting characters can be valid in some languages and emoji; review each finding in context. This is not a complete Unicode confusable-name detector.\n\nCore name and alias setters reject blank names and hidden controls before saving. Description setters allow newlines and tabs but reject hidden controls. The shared checks are $str_proto:name_issues, validate_name, validate_aliases, and validate_description. Direct programmer assignments can bypass these setters.\n\nThe audit reads stored values without calling object naming code. It does not rename objects or change aliases. Use an object ID with @rename after review.", .summary = "Audit all object names and aliases", .aliases = {"audit-names", "name audit"}, .category = 'administration, .content_kind = 'djot_blocks, .see_also = {"@rename", "@grammar"}>;
  property topic_wizard_builder (owner: ARCH_WIZARD, flags: "rc") = <HELP, .name = "@builder", .content = "Usage: `@builder <player>`\n\nGrants builder privileges (or downgrades programmer to builder).", .summary = "Grant builder status", .aliases = {"builder", "grant builder"}, .category = 'administration, .content_kind = 'djot_blocks, .see_also = {"@programmer"}>;
  property topic_wizard_chown (owner: ARCH_WIZARD, flags: "rc") = <HELP, .name = "@chown", .content = "Usage:\n\n- `@chown <object> to <owner>`\n- `@chown <object>.<property> to <owner>`\n- `@chown <object>:<verb> to <owner>`", .summary = "Change ownership", .aliases = {"chown", "owner"}, .category = 'administration, .content_kind = 'djot_blocks, .see_also = {}>;
  property topic_wizard_llm_budget (owner: ARCH_WIZARD, flags: "rc") = <HELP, .name = "@llm-budget", .content = "Usage: `@llm-budget <player>`\n\nShows token budget and usage data for a player.", .summary = "Show LLM budget", .aliases = {"llm-budget", "token budget"}, .category = 'administration, .content_kind = 'djot_blocks, .see_also = {"@llm-set-budget", "@llm-reset-usage"}>;
  property topic_wizard_llm_config (owner: ARCH_WIZARD, flags: "rc") = <HELP, .name = "@llm-config", .content = "Usage:\n\n- `@llm-config`\n- `@llm-config endpoint <url>`\n- `@llm-config model <name>`\n- `@llm-config key`", .summary = "Configure LLM client", .aliases = {"llm-config", "llm settings"}, .category = 'administration, .content_kind = 'djot_blocks, .see_also = {"@llm-reset-agents"}>;
  property topic_wizard_llm_reset_agents (owner: ARCH_WIZARD, flags: "rc") = <HELP, .name = "@llm-reset-agents", .content = "Usage: `@llm-reset-agents`\n\nReconfigures wearable and observer LLM agent instances.", .summary = "Reconfigure LLM agents", .aliases = {"llm-reset-agents", "llm agents"}, .category = 'administration, .content_kind = 'djot_blocks, .see_also = {"@llm-config"}>;
  property topic_wizard_llm_reset_usage (owner: ARCH_WIZARD, flags: "rc") = <HELP, .name = "@llm-reset-usage", .content = "Usage: `@llm-reset-usage <player>`\n\nResets token usage counter and usage log for a player.", .summary = "Reset LLM usage", .aliases = {"llm-reset-usage", "reset token usage"}, .category = 'administration, .content_kind = 'djot_blocks, .see_also = {"@llm-budget"}>;
  property topic_wizard_llm_set_budget (owner: ARCH_WIZARD, flags: "rc") = <HELP, .name = "@llm-set-budget", .content = "Usage: `@llm-set-budget <player> to <budget>`\n\nSets token budget for a player.", .summary = "Set LLM budget", .aliases = {"llm-set-budget", "set token budget"}, .category = 'administration, .content_kind = 'djot_blocks, .see_also = {"@llm-budget"}>;
  property topic_wizard_overview (owner: ARCH_WIZARD, flags: "rc") = <HELP, .name = "wizard", .content = "Wizard/admin command surface:\n\n`@announce`, `@programmer`, `@builder`, `@chown`, `@audit-names`, `@shutdown`, `@reconfigure-tools`, `@reissue-tools`, `@llm-budget`, `@llm-set-budget`, `@llm-reset-usage`, `@llm-config`, `@llm-reset-agents`\n\nUse `help <command>` for details.", .summary = "Wizard administration commands", .aliases = {"wizard", "admin"}, .category = 'administration, .content_kind = 'djot_blocks, .see_also = {"administration"}>;
  property topic_wizard_programmer (owner: ARCH_WIZARD, flags: "rc") = <HELP, .name = "@programmer", .content = "Usage: `@programmer <player>`\n\nGrants programmer privileges (or upgrades builder to programmer) and issues required tools.", .summary = "Grant programmer status", .aliases = {"programmer", "grant programmer"}, .category = 'administration, .content_kind = 'djot_blocks, .see_also = {"@builder"}>;
  property topic_wizard_reconfigure_tools (owner: ARCH_WIZARD, flags: "rc") = <HELP, .name = "@reconfigure-tools", .content = "Usage: `@reconfigure-tools`\n\nReconfigures all Architect's Compass and Data Visor instances.", .summary = "Reconfigure tool instances", .aliases = {"reconfigure-tools", "tool reconfigure"}, .category = 'administration, .content_kind = 'djot_blocks, .see_also = {"@reissue-tools"}>;
  property topic_wizard_reissue_tools (owner: ARCH_WIZARD, flags: "rc") = <HELP, .name = "@reissue-tools", .content = "Usage: `@reissue-tools`\n\nDestroys existing tool instances and reissues them to qualified players.", .summary = "Reissue tool instances", .aliases = {"reissue-tools", "tool reissue"}, .category = 'administration, .content_kind = 'djot_blocks, .see_also = {"@reconfigure-tools"}>;
  property topic_wizard_shutdown (owner: ARCH_WIZARD, flags: "rc") = <HELP, .name = "@shutdown", .content = "Usage: `@shutdown [in <minutes>] [message]`\n\nSends countdown announcements, dumps database, and shuts down server.", .summary = "Shutdown server", .aliases = {"shutdown", "reboot"}, .category = 'administration, .content_kind = 'djot_blocks, .see_also = {"@announce"}>;

  override topic_order (owner: ARCH_WIZARD, flags: "rc") = {
    'topic_wizard_overview,
    'topic_wizard_audit_names,
    'topic_wizard_announce,
    'topic_wizard_programmer,
    'topic_wizard_builder,
    'topic_wizard_chown,
    'topic_wizard_shutdown,
    'topic_wizard_reconfigure_tools,
    'topic_wizard_reissue_tools,
    'topic_wizard_llm_budget,
    'topic_wizard_llm_set_budget,
    'topic_wizard_llm_reset_usage,
    'topic_wizard_llm_config,
    'topic_wizard_llm_reset_agents
  };
endobject
