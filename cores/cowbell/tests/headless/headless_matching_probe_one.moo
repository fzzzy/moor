// Copyright (C) 2026 The mooR Authors
// SPDX-License-Identifier: GPL-3.0-or-later
object HEADLESS_MATCHING_PROBE_ONE [
  import_export_id -> "headless_matching_probe_one"
]
  name: "matching probe"
  parent: THING
  owner: RUNTIME_WIZARD
  location: RUNTIME_ROOM
  override aliases = {"first probe", "probe"};
endobject
