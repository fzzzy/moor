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
object RUNTIME_BASELINE [
  import_export_id -> "runtime_baseline"
]
  name: "Runtime Baseline Scenarios"
  parent: ROOT
  owner: RUNTIME_WIZARD
  readable: true

  method test_fixture_principals owner: RUNTIME_WIZARD
    $test_utils:assert_true(#90100.wizard, "fixture wizard flag");
    $test_utils:assert_true(#90100.programmer, "wizard programmer flag");
    $test_utils:assert_false(#90101.wizard, "programmer has no wizard flag");
    $test_utils:assert_true(#90101.programmer, "fixture programmer flag");
    $test_utils:assert_false(#90102.wizard, "player has no wizard flag");
    $test_utils:assert_false(#90102.programmer, "player has no programmer flag");
    $test_utils:assert_eq(#90100:permission_probe(), {1, 2}, "wizard protected write succeeds");
    $test_utils:assert_eq(#90101:permission_probe(), {1, E_PERM}, "programmer own write succeeds and foreign write denied");
    $test_utils:assert_eq(#90102:permission_probe(), {1, E_PERM}, "player own write succeeds and foreign write denied");
    for principal in ({#90100, #90101, #90102})
      $test_utils:assert_true(is_player(principal), "fixture player flag");
      $test_utils:assert_eq(principal.owner, principal, "fixture self ownership");
      $test_utils:assert_eq(principal.location, #90103, "isolated fixture room");
    endfor
    return true;
  endmethod
endobject
