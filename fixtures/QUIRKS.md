# Quirk coverage for 26.3

Phase 2's gate asks for a passing fixture for every quirk in the research
registry that is present in 26.3. This table says which fixtures cover each
one, or why none does yet. Every fixture listed is captured from the game in
all eight orientations and must match at the order level
(`crates/engine/tests/expected.txt`).

| Quirk | In 26.3 | Fixtures | Status |
| --- | --- | --- | --- |
| `qc` quasi-connectivity | yes | `phase1/piston_qc_diagonal`, `phase2/dropper_qc_bud` | covered for pistons and droppers (dispensers share the dropper's trigger); the crafter, which is not QC, is not simulated yet |
| `bud` stale QC power | yes | `phase1/piston_qc_bud`, `phase2/dropper_qc_bud` | covered |
| `piston-start-delay` | yes | `phase1/piston_from_repeater`, `phase1/piston_push` | covered |
| `zero-tick-pulse` | yes | `phase1/piston_zero_tick`, `phase1/sticky_piston_zero_tick` | covered |
| `block-spitting` | yes | `phase1/sticky_piston_pulse_1`, `phase1/sticky_piston_repeater_pulse` | covered |
| `zero-tick-farms` | no | | not present in 26.3 |
| `instant-tile-ticks` | no (believed) | | not present in 26.3 |
| `dust-locational` | yes | `phase2/dust_piston_race_a`, `phase2/dust_piston_race_b`, `nucleation/wire_corner_pull` | covered: the same race at two origins |
| `directional-update-order` | yes | `phase2/pistons_face_off` | covered |
| `redstone-experiments` | opt-in | | needs the experimental dust evaluator and an experiment-enabled capture |
| `dust-dot-cross` | yes | `phase1/dust_dot_vs_cross` | covered |
| `diode-pulse-loss` | yes | `phase1/repeater_chain_pulse_1` | covered |
| `observer-double-schedule` | yes | `phase2/observer_clock`, `phase2/observer_removed_mid_pulse` | covered |
| `observer-piston` | yes | `phase2/observer_sticky_piston` | covered |
| `torch-burnout` | yes | `phase1/torch_burnout_7`, `phase1/torch_burnout_8`, `phase1/torch_burnout_9`, `phase1/torch_clock_burnout` | covered |
| `update-suppression-so` | no | | not present in 26.3 |
| `update-skipping` | yes | | needs a chain longer than the 1,000,000-update default cap; to do |
| `update-suppression-other` | no | | not present in 26.3 |
| `tnt-dupe` | yes, captured | `phase4/tnt_dupe_coral` | covered: the primed TNT and the placed TNT block both appear in 26.3 |
| `rail-carpet-dupe` | yes, captured | `phase4/carpet_dupe`, `phase4/rail_dupe` | covered: in 26.3 the dropped carpet or rail and the placed copy both appear. The carpet pops when the coral fan under it does; the rail drops when the fan's pop updates it while a moving piston, not yet solid, stands under it |
| `gravity-dupe` | unverified | | not simulated: it needs end portals, dimension travel and an entity riding in the portal, none of which the engine has. A build with an end portal is refused by name |
| `string-dupe` | no | | not present in 26.3 |
| `flying-machines` | yes | `nucleation/flying_kick`, `nucleation/slime_extender` | covered |
| `block-entities-immovable` | yes | `phase1/piston_block_entity`, `phase1/piston_immovable` | covered |
| `push-limit` | yes | `phase1/piston_push_limit_12`, `phase1/piston_push_limit_13` | covered |
| `headless-pistons` | yes | | needs TNT (phase 4) |
| `powered-rail-order` | yes | `phase2/powered_rail_slope`, `phase2/activator_rail_chain`, `nucleation/rails_chain` | covered |
| `minecart-improvements` | opt-in | | not simulated: the engine runs the default minecart physics (`OldMinecartBehavior`), which 26.3 worlds use unless the experiment is on |
| `crafter-no-qc` | yes | | the crafter is not simulated yet |
| `be-tick-order` | yes | `phase2/hoppers_share_hopper`, `phase2/hopper_chain_cooldown`, `nucleation/hopper_race` | covered |
| `comparator-quirks` | unverified | `phase2/comparator_reads_composter`, `phase2/comparator_reads_cake`, `phase2/hopper_chest_to_chest` | covered for composters, cakes and blocked chests |
| `item-shadowing` | no | | not present in 26.3 |
