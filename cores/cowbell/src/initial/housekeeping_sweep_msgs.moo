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
      <SUB, .capitalize = false, .binding_name = 'actor, .type = 'article_the>,
      " back to bed."
    },
    {
      "Housekeeping escorts ",
      <SUB, .capitalize = false, .binding_name = 'actor, .type = 'article_the>,
      " away to ",
      <SUB, .capitalize = false, .type = 'pos_adj>,
      " room."
    },
    {
      <SUB, .capitalize = true, .binding_name = 'actor, .type = 'article_the>,
      " ",
      <SUB, .binding_name = 'actor, .type = 'agree, .subject_form = 'name, .plural_form = "are", .singular_form = "is">,
      " quietly led away by a tired-looking porter."
    },
    {
      "A woman with a broom nudges ",
      <SUB, .capitalize = false, .binding_name = 'actor, .type = 'article_the>,
      " awake and points toward the back stairs."
    },
    {
      <SUB, .capitalize = true, .binding_name = 'actor, .type = 'article_the>,
      " ",
      <SUB, .binding_name = 'actor, .type = 'agree, .subject_form = 'name, .plural_form = "are", .singular_form = "is">,
      " gently but firmly ushered toward the dormitory."
    }
  };
endobject
