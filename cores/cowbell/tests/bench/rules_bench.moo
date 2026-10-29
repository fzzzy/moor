object RULES_BENCH [
  import_export_id -> "rules_bench"
]
  name: "Rules Engine Bench"
  parent: core::ROOT
  owner: core::HACKER
  readable: true

  override description (owner: core::HACKER, flags: "rc") = "Timings for conditions and rules. Run with make bench-rules; the normal method suite does not load this package.";

  method _measure owner: core::ARCH_WIZARD
    "Count the ticks of one call, then time calls until the budget is spent. Log and return both.";
    const {name, work, ?budget = 0.5} = args;
    suspend(0);
    const before = ticks_left();
    work();
    const ticks = before - ticks_left();
    let calls = 0;
    let elapsed = 0.0;
    while (elapsed < budget || calls < 5)
      if (ticks_left() < ticks + 1000 || seconds_left() < 1)
        suspend(0);
      endif
      const start = ftime();
      work();
      elapsed = elapsed + (ftime() - start);
      calls = calls + 1;
    endwhile
    const per_call = elapsed * 1000.0 / tofloat(calls);
    server_log("BENCH_DATA bench=rules case=" + name + " ticks=" + tostr(ticks) + " per_call_ms=" + floatstr(per_call, 4) + " calls=" + tostr(calls));
    return ['case -> name, 'ticks -> ticks, 'per_call_ms -> per_call, 'calls -> calls];
  endmethod

  method _rows owner: core::HACKER
    "Fixed predicate rows: a thing and a score for each.";
    const {count} = args;
    return { ['thing -> toobj(i), 'score -> (i * 7919) % 1000] for i in [1..count] };
  endmethod

  method _provider owner: core::HACKER
    "A provider answering every predicate with fixed rows, honouring the limit as real providers do.";
    const {rows} = args;
    return {name, values, limit} => ['rows -> rows[1..min(length(rows), limit)], 'complete -> length(rows) <= limit];
  endmethod

  method _env owner: core::HACKER
    "An environment with only inputs.";
    const {?inputs = []} = args;
    return ['input -> inputs, 'parameter -> [], 'state -> [], 'binding -> []];
  endmethod

  method test_bench_conditions owner: core::ARCH_WIZARD
    "Condition evaluation without validation, against fixed rows.";
    const env = this:_env(['P -> #0, 'A -> 3]);
    const near = "near(place: input.P) -> (thing: t, score: s)";
    const cases = {{"compare", "input.A + 1 > 2", 1}, {"predicate_1", near, 1}, {"predicate_16", near, 16}, {"predicate_256", near, 256}, {"join_shared_16", near + " AND near(place: input.P) -> (thing: t, score: s2)", 16}, {"join_shared3_16", near + " AND near(place: input.P) -> (thing: t, score: s2) AND near(place: input.P) -> (thing: t, score: s3)", 16}, {"cross_8x8", "near(place: input.P) -> (thing: a) AND near(place: input.P) -> (thing: b)", 8}, {"cross_16x16", "near(place: input.P) -> (thing: a) AND near(place: input.P) -> (thing: b)", 16}, {"filter_256", near + " AND binding.s < 500", 256}, {"count_256", "count(" + near + ") > 3", 256}, {"best_256", "best(" + near + ", by: binding.s, limit: 3)", 256}};
    for spec in (cases)
      const {name, text, count} = spec;
      const tree = $condition:parse(text);
      const provider = this:_provider(this:_rows(count));
      this:_measure(name, {} => $condition:evaluate(tree, env, provider));
    endfor
    const joined = $condition:parse(near + " AND near(place: input.P) -> (thing: t, score: s2) AND binding.s2 > 900");
    const provider = this:_provider(this:_rows(16));
    this:_measure("explain_off_16", {} => $condition:evaluate(joined, env, provider));
    this:_measure("explain_on_16", {} => $condition:evaluate(joined, env, provider, 256, true));
    return true;
  endmethod

  method test_bench_rules owner: core::ARCH_WIZARD
    "Whole rule evaluations, which validate the rule on every call, against the world and relations.";
    const simple = $rule_engine:parse_expression("input.A + 1 > 2", 'simple, $rule_engine:scope(['A -> 'integer]));
    this:_measure("rule_compare", {} => $rule_engine:evaluate(simple, ['A -> 3]));
    this:_measure("parse_compare", {} => $condition:parse("input.A + 1 > 2"));
    this:_measure("validate_compare", {} => $rule_engine:validate_rule(simple));
    const room = create($room, $hacker, 2);
    let relations = {};
    try
      for i in [1..16]
        move(create($thing, $hacker, 2), room);
      endfor
      const occupants = $rule_engine:parse_expression("occupants(location: input.Location) -> (object: Visitor)", 'occupants, $rule_engine:scope(['Location -> 'object]));
      this:_measure("rule_occupants_16", {} => $rule_engine:evaluate(occupants, ['Location -> room]));
      for size in ({100, 10000})
        const relation = create($relation, $hacker, 2);
        relations = {@relations, relation};
        "Fields come first: define_fields checks every stored fact in one task.";
        $relation:define_fields(relation, {'group, 'member}, ['group -> 'integer, 'member -> 'integer]);
        let start = ftime();
        for i in [1..size]
          ticks_left() < 1000000 && suspend(0);
          const before = ticks_left();
          relation:assert({i % (size / 10), i});
          if (i == size)
            server_log("BENCH_DATA bench=rules case=assert_" + tostr(size) + " ticks=" + tostr(before - ticks_left()) + " per_call_ms=" + floatstr((ftime() - start) * 1000.0 / tofloat(i), 4) + " calls=" + tostr(i));
          endif
        endfor
        const rule = $rule_engine:parse_expression("relation(" + tostr(relation) + ", group: 7) -> (member: M)");
        this:_measure("rule_relation_" + tostr(size), {} => $rule_engine:evaluate(rule));
      endfor
    finally
      for relation in (relations)
        relation:destroy();
      endfor
      for thing in (room.contents)
        thing:destroy();
      endfor
      room:destroy();
    endtry
    return true;
  endmethod

endobject
