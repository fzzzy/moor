object HOUSEKEEPING_SWEEP_MSGS [
  import_export_id -> "housekeeping_sweep_msgs",
  import_export_hierarchy -> {"initial"}
]
  name: "Housekeeping Sweep Messages"
  parent: MSG_BAG
  owner: ARCH_WIZARD

  override entries (owner: ARCH_WIZARD, flags: "rc") = {
    {
      "A member of housekeeping gently guides ",
      <SUB, .binding_name = 'actor, .type = 'article_the, .capitalize = false>,
      " back to bed."
    },
    {
      "Housekeeping escorts ",
      <SUB, .binding_name = 'actor, .type = 'article_the, .capitalize = false>,
      " away to ",
      <SUB, .type = 'pos_adj, .capitalize = false>,
      " room."
    },
    {
      <SUB, .binding_name = 'actor, .type = 'article_the, .capitalize = true>,
      " ",
      <SUB, .binding_name = 'actor, .plural_form = "are", .type = 'agree, .subject_form = 'name, .singular_form = "is">,
      " quietly led away by a tired-looking porter."
    },
    {
      "A woman with a broom nudges ",
      <SUB, .binding_name = 'actor, .type = 'article_the, .capitalize = false>,
      " awake and points toward the back stairs."
    },
    {
      <SUB, .binding_name = 'actor, .type = 'article_the, .capitalize = true>,
      " ",
      <SUB, .binding_name = 'actor, .plural_form = "are", .type = 'agree, .subject_form = 'name, .singular_form = "is">,
      " gently but firmly ushered toward the dormitory."
    }
  };
endobject
