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
object RUNTIME_PLAYER [
  import_export_id -> "runtime_player"
]
  name: "RUNTIME_PLAYER"
  parent: cowbell::PLAYER
  location: RUNTIME_ROOM
  owner: RUNTIME_PLAYER
  player: true
  readable: true

  property runtime_probe (owner: RUNTIME_PLAYER, flags: "r") = 0;

  override home (owner: RUNTIME_PLAYER, flags: "rc") = RUNTIME_ROOM;

  verb "runtime-test-raise" (none none none) owner: RUNTIME_WIZARD flags: "rd"
    "Raise an uncaught error to verify command exception handling.";
    raise(E_INVARG, "cowbell harness command exception");
  endverb

  method permission_probe owner: RUNTIME_PLAYER
    "Prove own writes succeed and report a protected foreign write.";
    this.runtime_probe = 1;
    const own_result = this.runtime_probe;
    const foreign_result = `#90100.runtime_probe = 2 ! E_PERM => E_PERM';
    return {own_result, foreign_result};
  endmethod

  verb "runtime-test-denial" (none none none) owner: RUNTIME_PLAYER flags: "rd"
    "Emit an ordinary denial without an exception.";
    this:inform_current($event:mk_error(this, "Fixture denial"));
  endverb
endobject
