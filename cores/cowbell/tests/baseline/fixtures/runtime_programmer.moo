// Copyright (C) 2026 Ryan Daum <ryan.daum@gmail.com>
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
object RUNTIME_PROGRAMMER [
  import_export_id -> "runtime_programmer"
]
  name: "RUNTIME_PROGRAMMER"
  parent: PLAYER
  owner: RUNTIME_PROGRAMMER
  location: RUNTIME_ROOM
  player: true
  programmer: true
  readable: true
  override import_export_id = "runtime_programmer";
  override home = RUNTIME_ROOM;
  property runtime_probe (owner: RUNTIME_PROGRAMMER, flags: "r") = 0;
  verb permission_probe (this none this) owner: RUNTIME_PROGRAMMER flags: "rxd"
    "Prove own writes succeed and report a protected foreign write.";
    this.runtime_probe = 1;
    const own_result = this.runtime_probe;
    const foreign_result = `(#90100.runtime_probe = 2) ! E_PERM => E_PERM';
    return {own_result, foreign_result};
  endverb
endobject
