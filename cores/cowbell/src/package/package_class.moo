object PACKAGE_CLASS [
  import_export_id -> "package_class"
]
  name: "Generic Package"
  owner: core::ARCH_WIZARD
  readable: true

  property depends (owner: core::ARCH_WIZARD, flags: "rc") = {};
  property initialized_version (owner: core::ARCH_WIZARD, flags: "r") = 0;
  property version (owner: core::ARCH_WIZARD, flags: "rc") = 0;

  method package_name owner: core::ARCH_WIZARD
    "The name of the package this object stands for, from its objdef_package metadata.";
    const name = object_metadata(this, 'objdef_package);
    return typeof(name) == TYPE_STR ? name | "";
  endmethod

  method package_objects owner: core::ARCH_WIZARD
    "The objects that belong to this object's package.";
    const name = this:package_name();
    return name ? objects_with_metadata('objdef_package, name) | {};
  endmethod

  method installed owner: core::ARCH_WIZARD
    "The package objects in the world, each after the packages it depends on, then by name.";
    let remaining = [];
    for object in (descendants(this))
      const name = object:package_name();
      name && !maphaskey(remaining, name) && (remaining[name] = object);
    endfor
    let ordered = {};
    let placed = {};
    while (remaining)
      let ready = {};
      for name in (mapkeys(remaining))
        const waiting = { depend for depend in (remaining[name].depends) if maphaskey(remaining, depend) && depend != name };
        waiting || (ready = {@ready, name});
      endfor
      "A cycle leaves nothing ready; place the rest by name rather than stall.";
      ready || (ready = mapkeys(remaining));
      for name in (sort(ready))
        ordered = {@ordered, remaining[name]};
        remaining = mapdelete(remaining, name);
      endfor
    endwhile
    return ordered;
  endmethod

  method sync owner: core::ARCH_WIZARD
    "Run init() for each package whose version changed since it last ran, then refresh() for all, in dependency order.";
    callers() && !caller_perms().wizard && raise(E_PERM);
    for package in (this:installed())
      try
        if (package.initialized_version != package.version)
          package:init();
          package.initialized_version = package.version;
        endif
        package:refresh();
      except error (ANY)
        server_log(tostr("Package ", package:package_name(), " (", package, ") failed to start: ", toliteral(error[1..2])));
      endtry
    endfor
  endmethod

  method init owner: core::ARCH_WIZARD
    "Set up the package after it is installed or its version changes.";
    return;
  endmethod

  method refresh owner: core::ARCH_WIZARD
    "Restore the package's running state when the server starts.";
    return;
  endmethod

  method new_player owner: core::ARCH_WIZARD
    "Set up a newly created player.";
    return;
  endmethod

  method player_features owner: core::ARCH_WIZARD
    "The feature objects this package adds to the player's commands and help.";
    return {};
  endmethod

  method help_sources owner: core::ARCH_WIZARD
    "The objects this package adds to the player's help topics.";
    return {};
  endmethod

  method agent_tools owner: core::ARCH_WIZARD
    "Tool definitions this package offers external agents.";
    return {};
  endmethod

  method agent_resources owner: core::ARCH_WIZARD
    "Resources this package offers external agents.";
    return {};
  endmethod

  method named owner: core::ARCH_WIZARD
    "The installed package object for a package name, or #-1.";
    const {name} = args;
    typeof(name) == TYPE_STR && name || return #-1;
    for object in (objects_with_metadata('objdef_package, name))
      if (object != $package && isa(object, $package))
        return object;
      endif
    endfor
    return #-1;
  endmethod

  method client_data owner: core::ARCH_WIZARD
    "Structured data a client sent in this package's namespace, from #0:do_client_data; player is the sender. Implementations should check that caller is #0. Ignored by default.";
    return;
  endmethod

  method host_call owner: core::ARCH_WIZARD
    "A host's own call to this package, from #0:invoke_package_handler; player is the acting player, or #0 when the host acts for itself. Implementations should check that caller is #0. Refused by default.";
    raise(E_INVARG, tostr("Package ", this:package_name(), " takes no host calls."));
  endmethod

  method test_installed_follows_depends owner: core::ARCH_WIZARD
    "Packages come after those they depend on, whatever their names.";
    const later = create(this);
    const earlier = create(this);
    try
      set_object_metadata(later, 'objdef_package, "zz_test_aaa");
      set_object_metadata(earlier, 'objdef_package, "zz_test_zzz");
      later.depends = {"zz_test_zzz"};
      const order = this:installed();
      $test_utils:assert_true(earlier in order && later in order, "both packages are installed");
      $test_utils:assert_true(earlier in order < (later in order), "a dependency comes first");
      $test_utils:assert_eq(later:package_objects(), {later}, "a package lists its tagged objects");
    finally
      recycle(later);
      recycle(earlier);
    endtry
    return true;
  endmethod

  method test_named_finds_the_package_object owner: core::ARCH_WIZARD
    "A package name finds its package object, not the package's other objects.";
    const package = create(this);
    const member = create(#-1);
    try
      set_object_metadata(package, 'objdef_package, "zz_test_named");
      set_object_metadata(member, 'objdef_package, "zz_test_named");
      $test_utils:assert_eq(this:named("zz_test_named"), package, "the package object");
      $test_utils:assert_eq(this:named("zz_test_absent"), #-1, "no such package");
      $test_utils:assert_eq(this:named(""), #-1, "no name");
      $test_utils:assert_raises(E_INVARG, package, "host_call", {"anything"}, "host calls are refused by default");
    finally
      recycle(package);
      recycle(member);
    endtry
    return true;
  endmethod

  method test_sync_runs_init_once_per_version owner: core::ARCH_WIZARD
    "init() runs when the version changes; refresh() runs on every sync.";
    const package = create(this);
    try
      set_object_metadata(package, 'objdef_package, "zz_test_sync");
      add_property(package, "inits", 0, {$arch_wizard, "r"});
      add_property(package, "refreshes", 0, {$arch_wizard, "r"});
      add_verb(package, {$arch_wizard, "rxd", "init"}, {"this", "none", "this"});
      set_verb_code(package, "init", {"this.inits = this.inits + 1;"});
      add_verb(package, {$arch_wizard, "rxd", "refresh"}, {"this", "none", "this"});
      set_verb_code(package, "refresh", {"this.refreshes = this.refreshes + 1;"});
      package.version = 1;
      this:sync();
      this:sync();
      $test_utils:assert_eq(package.inits, 1, "init runs once for a version");
      $test_utils:assert_eq(package.refreshes, 2, "refresh runs on every sync");
      package.version = 2;
      this:sync();
      $test_utils:assert_eq(package.inits, 2, "a new version runs init again");
    finally
      recycle(package);
    endtry
    return true;
  endmethod
endobject
