# Community builds

Well-known redstone designs from the Java community, rebuilt block by
block from the Minecraft Wiki's schematics of them and credited to the
people they are known by. Mechanics are free to use, but the designs belong
to the people who made them, so each fixture names who it is credited to
and the wiki page its layout follows. No schematic file was downloaded or
copied, and the credits don't imply that anyone endorses Tickwright.

Each build is captured from the real 26.3 server in all eight orientations
and matches at the order level (`crates/engine/tests/expected.txt`).

| Fixture | Build | Credited to | Source | What the fixture shows |
| --- | --- | --- | --- | --- |
| `jeb_door` | Jeb door, a 2x2 flush piston door | Jens "Jeb" Bergensten, who first showed it | [sticky piston preview](https://www.youtube.com/watch?v=uZJr86d2IUo); [wiki schematic](https://minecraft.wiki/w/Tutorial:Piston_uses/Full_Jeb_door) | A lever closes the door, the stone blocks ending flush with the wall's face, then opens it, twice. The lower six of its twelve sticky pistons fire by quasi-connectivity |
| `etho_hopper_clock` | Etho hopper clock | EthosLab | [Minecraft - Tutorial: Hopper Timer](https://www.youtube.com/watch?v=bYwzd62cLSw); [wiki schematic](https://minecraft.wiki/w/Redstone_circuits/Clock/Hopper_clocks) | Three items pass back and forth between two hoppers while two sticky pistons hand a redstone block between them, every 36 ticks |
| `grizdale_tff` | Compact T flip-flop (variant A) | Grizdale | [wiki schematic](https://minecraft.wiki/w/Redstone_circuits/Memory/TFF_Grizdale) | Four button presses move one stick between two droppers, by way of a chest and a hopper, turning a lamp on, off, on and off |
| `ilmango_shulker_sorter` | Potion sorter, with a shulker box as the filter | ilmango | [Minecraft Potion Sorter](https://www.youtube.com/watch?v=1-oKvnnUX4c); [wiki schematic](https://minecraft.wiki/w/Tutorial:Hopper#Alternate_version); [shulker box as the filter](https://minecraft.wiki/w/Tutorial:Hopper#Potions,_books_and_shulker_boxes) | Four cobblestone pass through the shulker box into one chest; the two shulker boxes, which a shulker box refuses, go to the other |
| `firewolf_flying_machine` | Driveable four-way flying machine D | FirewolfQuasar | [4 way flying machine](https://www.youtube.com/watch?v=MnppZ2pRxYI); [wiki schematic](https://minecraft.wiki/w/Tutorial:Flying_machines#Driveable_flying_machine_D,_4-way) | A torch placed beside the top observer and broken starts the machine; placed and left, it stops it. It flies five blocks east, stops, flies five blocks back west and stops |

Notes on the rebuilds:

- The Jeb door's input is a lever on the back of the block under the
  dust, which the schematic leaves open.
- The Etho clock's items start in one hopper with both pistons retracted
  and the redstone block beside the left hopper, so the clock starts
  itself, as it does when a player fills it. The wiki's picture shows the
  left piston extended and the block over the right hopper.
- The T flip-flop's output dust sits on a lamp, so its state can be seen.
- The flying machine is built without its rider; a fixture can't place a
  player.
