// Copyright (C) 2026 The mooR Authors
// SPDX-License-Identifier: GPL-3.0-or-later
object HEADLESS_MATCHING_PROBE_ONE [
  import_export_id -> "headless_matching_probe_one"
]
  name: "matching probe"
  parent: THING
  location: fixtures::RUNTIME_ROOM
  owner: fixtures::RUNTIME_WIZARD

  override aliases (owner: fixtures::RUNTIME_WIZARD, flags: "rc") = {"first probe", "probe"};
endobject
