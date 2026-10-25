// Copyright (C) 2026 The mooR Authors
// SPDX-License-Identifier: GPL-3.0-or-later
object HEADLESS_MATCHING_UUID_PROBE [
  import_export_id -> "headless_matching_uuid_probe"
]
  name: "UUID matching probe"
  parent: THING
  location: fixtures::RUNTIME_ROOM
  owner: fixtures::RUNTIME_WIZARD

  verb "runtime-match-probe" (this none none) owner: fixtures::RUNTIME_WIZARD flags: "rd"
    "Expose dispatch through an actual UUID object reference.";
    player:inform_current($event:mk_note(player, "MATCHING DISPATCH " + tostr(this)):with_audience('utility));
  endverb
endobject
