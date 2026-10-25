// Copyright (C) 2026 The mooR Authors
// SPDX-License-Identifier: GPL-3.0-or-later
object HEADLESS_MATCHING_PROBE_TWO [
  import_export_id -> "headless_matching_probe_two"
]
  name: "matching probe"
  parent: cowbell::THING
  location: fixtures::RUNTIME_ROOM
  owner: fixtures::RUNTIME_WIZARD

  override aliases (owner: fixtures::RUNTIME_WIZARD, flags: "rc") = {"second probe", "probe", "special probe"};

  verb "runtime-match-probe" (this none none) owner: fixtures::RUNTIME_WIZARD flags: "rd"
    "Expose the real object selected by command dispatch.";
    player:inform_current($event:mk_note(player, "MATCHING DISPATCH " + tostr(this)):with_audience('utility));
  endverb
endobject
