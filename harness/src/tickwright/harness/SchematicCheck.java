package tickwright.harness;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.mojang.serialization.Dynamic;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Optional;
import java.util.TreeMap;
import java.util.UUID;
import java.util.function.BooleanSupplier;
import java.util.stream.Stream;
import net.minecraft.SharedConstants;
import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.Vec3i;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.gametest.framework.GameTestServer;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.NbtAccounter;
import net.minecraft.nbt.NbtIo;
import net.minecraft.nbt.NbtOps;
import net.minecraft.nbt.NbtUtils;
import net.minecraft.nbt.StringTag;
import net.minecraft.nbt.Tag;
import net.minecraft.nbt.TagParser;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.TicketType;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.util.ProblemReporter;
import net.minecraft.util.datafix.DataFixTypes;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.util.datafix.fixes.References;
import net.minecraft.world.Container;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntitySpawnRequest;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.item.FallingBlockEntity;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.entity.vehicle.minecart.AbstractMinecart;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.ComparatorBlockEntity;
import net.minecraft.world.level.block.piston.MovingPistonBlock;
import net.minecraft.world.level.block.piston.PistonMovingBlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.structure.templatesystem.LiquidSettings;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructurePlaceSettings;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplate;
import net.minecraft.world.level.storage.LevelStorageSource;
import net.minecraft.world.level.storage.TagValueInput;
import net.minecraft.world.phys.AABB;

/**
 * Loads schematic files the engine wrote into a 26.3 level, and checks the
 * level then holds what they were written from, block for block.
 *
 * <p>Structure {@code .nbt} files go through the game's own loader: its data
 * fixer from the file's data version ({@code TemplateSource.readStructure}),
 * then {@link StructureTemplate#placeInWorld}. Sponge {@code .schem} and
 * {@code .litematic} files have no reader in the game, so this class unpacks
 * their block arrays as WorldEdit and Litematica do, and hands everything
 * else to the game: palette entries to its block state parser or
 * {@link NbtUtils#readBlockState} (Litematica 26.3 reads {@code id}, or
 * {@code Name} when there is none), and block entity and entity data,
 * fixed from the file's data version as both mods do, to the game's own
 * loaders.
 *
 * <p>Each case is a directory holding what the files must place, either a
 * resolved fixture ({@code expected.json}: its blocks, container contents,
 * comparator outputs and start entities, summoned as {@code /summon} does)
 * or a structure the game loads itself ({@code reference.nbt}), and any
 * number of {@code .nbt}, {@code .schem} and {@code .litematic} files.
 * Entities are compared by type, position and the block state they carry
 * (a falling block's, a minecart's display block), and block entities by
 * the data the game saves for them.
 *
 * <p>With {@code --save <file>} it first builds a small build in the level
 * (blocks, a chest, a comparator, a moving piston, falling blocks and a
 * minecart showing a block) and saves it as the game saves a structure, for
 * the engine to import.
 *
 * <p>Usage: {@code SchematicCheck --universe <dir> [--save <file>] [--cases <dir>]}
 */
public final class SchematicCheck {
    private static final int FLAGS = Block.UPDATE_CLIENTS | Block.UPDATE_KNOWN_SHAPE | Block.UPDATE_SKIP_ON_PLACE;
    // The largest fixture is 30 by 19 by 35 blocks; boxes start here, so with
    // a margin they stay within the same 4 by 4 chunks the capture uses.
    private static final BlockPos ORIGIN = new BlockPos(-20, 100, -20);
    private static int CURRENT;

    private static MinecraftServer server;
    private static ServerLevel level;
    private static HolderLookup<Block> blocks;
    private static Method tickServer;
    private static Method waitUntilNextTick;

    private SchematicCheck() {}

    public static void main(String[] args) throws Exception {
        String casesArg = arg(args, "--cases", null);
        Path universe = Path.of(arg(args, "--universe", "work/universe-schematics"));
        String save = arg(args, "--save", null);

        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        CURRENT = SharedConstants.getCurrentVersion().dataVersion().version();
        deleteRecursively(universe);
        LevelStorageSource.LevelStorageAccess access = LevelStorageSource.createDefault(universe).createAccess("check");
        PackRepository packs = ServerPacksSource.createPackRepository(access);
        server = GameTestServer.create(Thread.currentThread(), access, packs, Optional.empty(), false, 1);
        Method initServer = MinecraftServer.class.getDeclaredMethod("initServer");
        tickServer = MinecraftServer.class.getDeclaredMethod("tickServer", BooleanSupplier.class);
        waitUntilNextTick = MinecraftServer.class.getDeclaredMethod("waitUntilNextTick");
        initServer.setAccessible(true);
        tickServer.setAccessible(true);
        waitUntilNextTick.setAccessible(true);
        initServer.invoke(server);
        level = server.overworld();
        blocks = level.holderLookup(Registries.BLOCK);
        // Entities are found only in chunks loaded with their entities, so
        // load the area first. Nothing ticks after that.
        warmUp(40);
        System.out.printf("Minecraft %s, data version %d%n", SharedConstants.getCurrentVersion().name(), CURRENT);

        if (save != null) {
            saveSample(Path.of(save));
        }
        if (casesArg == null) {
            server.halt(false);
            System.exit(0);
        }
        Path cases = Path.of(casesArg);

        List<Path> dirs;
        try (Stream<Path> s = Files.list(cases)) {
            dirs = s.filter(Files::isDirectory).sorted().toList();
        }
        int files = 0;
        List<String> failures = new ArrayList<>();
        for (Path dir : dirs) {
            Snapshot expected = Files.exists(dir.resolve("expected.json"))
                    ? fromFixture(dir.resolve("expected.json"))
                    : fromStructure(dir.resolve("reference.nbt"));
            List<Path> candidates;
            try (Stream<Path> s = Files.list(dir)) {
                candidates = s.filter(p -> p.toString().matches(".*\\.(nbt|schem|litematic)$"))
                        .filter(p -> !p.getFileName().toString().equals("reference.nbt"))
                        .sorted()
                        .toList();
            }
            if (Boolean.getBoolean("check.verbose")) {
                System.out.printf("%s: %d block(s), %d block entit(ies), entities %s%n", dir.getFileName(),
                        expected.blocks.size(), expected.blockEntities.size(), expected.entities);
            }
            for (Path file : candidates) {
                files++;
                Snapshot got = load(file, expected.size);
                String diff = expected.diff(got);
                if (diff != null) {
                    failures.add(dir.getFileName() + "/" + file.getFileName() + ": " + diff);
                }
            }
        }
        System.out.printf("loaded %d file(s) in %d case(s): %d differ%n", files, dirs.size(), failures.size());
        failures.forEach(f -> System.out.println("  DIFFERS: " + f));
        server.halt(false);
        System.exit(failures.isEmpty() && files > 0 ? 0 : 1);
    }

    // --- what a case must place ---------------------------------------------

    /** Every block in the box, the block entities' saved data, and the entities. */
    private static final class Snapshot {
        Vec3i size;
        final TreeMap<String, String> blocks = new TreeMap<>();
        final TreeMap<String, String> blockEntities = new TreeMap<>();
        final List<String> entities = new ArrayList<>();

        String diff(Snapshot other) {
            if (!size.equals(other.size)) {
                return "size " + other.size + " where " + size + " was expected";
            }
            for (var e : blocks.entrySet()) {
                String got = other.blocks.get(e.getKey());
                if (!e.getValue().equals(got)) {
                    long wrong = blocks.keySet().stream().filter(k -> !blocks.get(k).equals(other.blocks.get(k))).count();
                    return wrong + " block(s) differ, first at " + e.getKey() + ": " + got + " where " + e.getValue()
                            + " was expected";
                }
            }
            for (var e : blockEntities.entrySet()) {
                String got = other.blockEntities.get(e.getKey());
                if (!e.getValue().equals(got)) {
                    return "block entity at " + e.getKey() + ": " + got + " where " + e.getValue() + " was expected";
                }
            }
            if (!entities.equals(other.entities)) {
                return "entities " + other.entities + " where " + entities + " were expected";
            }
            return null;
        }
    }

    /** What a resolved fixture places: blocks, contents, comparator outputs and start entities. */
    private static Snapshot fromFixture(Path file) throws Exception {
        JsonObject f = JsonParser.parseString(Files.readString(file)).getAsJsonObject();
        int[] size = ints(f.getAsJsonArray("size"));
        Snapshot s = new Snapshot();
        s.size = new Vec3i(size[0], size[1], size[2]);
        for (int y = 0; y < size[1]; y++) {
            for (int z = 0; z < size[2]; z++) {
                for (int x = 0; x < size[0]; x++) {
                    s.blocks.put(key(x, y, z), BlockStateParser.serialize(Blocks.AIR.defaultBlockState()));
                }
            }
        }
        for (JsonElement b : f.getAsJsonArray("blocks")) {
            JsonArray a = b.getAsJsonArray();
            BlockState state = BlockStateParser.parseForBlock(blocks, a.get(3).getAsString(), false).blockState();
            s.blocks.put(key(a.get(0).getAsInt(), a.get(1).getAsInt(), a.get(2).getAsInt()), BlockStateParser.serialize(state));
        }
        if (f.has("containers")) {
            for (JsonElement c : f.getAsJsonArray("containers")) {
                JsonObject o = c.getAsJsonObject();
                TreeMap<Integer, String> items = new TreeMap<>();
                for (JsonElement i : o.getAsJsonArray("items")) {
                    JsonObject it = i.getAsJsonObject();
                    String id = it.get("id").getAsString();
                    items.put(it.get("slot").getAsInt(), (id.contains(":") ? id : "minecraft:" + id) + "*" + it.get("count").getAsInt());
                }
                int[] p = ints(o.getAsJsonArray("pos"));
                s.blockEntities.put(key(p[0], p[1], p[2]), "items " + items);
            }
        }
        if (f.has("comparators")) {
            for (JsonElement c : f.getAsJsonArray("comparators")) {
                JsonObject o = c.getAsJsonObject();
                int[] p = ints(o.getAsJsonArray("pos"));
                s.blockEntities.put(key(p[0], p[1], p[2]), "output " + o.get("output").getAsInt());
            }
        }
        clear(s.size);
        if (f.has("start_entities")) {
            for (JsonElement e : f.getAsJsonArray("start_entities")) {
                JsonObject o = e.getAsJsonObject();
                CompoundTag tag = o.has("nbt") ? TagParser.parseCompoundFully(o.get("nbt").getAsString()) : new CompoundTag();
                String type = o.get("entity").getAsString();
                tag.putString("id", type.contains(":") ? type : "minecraft:" + type);
                JsonArray at = o.getAsJsonArray("at");
                Entity entity = EntityType.loadEntityRecursive(tag, level, new EntitySpawnRequest(EntitySpawnReason.COMMAND, false), en -> {
                    en.snapTo(ORIGIN.getX() + at.get(0).getAsDouble(), ORIGIN.getY() + at.get(1).getAsDouble(),
                            ORIGIN.getZ() + at.get(2).getAsDouble(), en.getYRot(), en.getXRot());
                    return en;
                });
                if (entity == null) {
                    throw new IllegalStateException(file + ": the game made no " + type + " from " + tag);
                }
                level.addFreshEntityWithPassengers(entity);
            }
        }
        s.entities.addAll(entities(s.size));
        return s;
    }

    /** What a structure the game loads itself places. */
    private static Snapshot fromStructure(Path file) throws Exception {
        return load(file, null);
    }

    // --- loading a file -----------------------------------------------------

    /** Clear the box, load the file into it, and record what it holds. */
    private static Snapshot load(Path file, Vec3i expectedSize) throws Exception {
        CompoundTag root = NbtIo.readCompressed(file, NbtAccounter.unlimitedHeap());
        String name = file.getFileName().toString();
        Vec3i size;
        if (name.endsWith(".nbt")) {
            size = placeStructure(root, expectedSize);
        } else if (name.endsWith(".schem")) {
            size = placeSponge(root, expectedSize);
        } else {
            size = placeLitematic(root, expectedSize);
        }
        Snapshot s = new Snapshot();
        s.size = size;
        for (int y = 0; y < size.getY(); y++) {
            for (int z = 0; z < size.getZ(); z++) {
                for (int x = 0; x < size.getX(); x++) {
                    BlockPos p = ORIGIN.offset(x, y, z);
                    s.blocks.put(key(x, y, z), BlockStateParser.serialize(level.getBlockState(p)));
                    BlockEntity be = level.getBlockEntity(p);
                    String d = be == null ? null : describe(be);
                    if (d != null) {
                        s.blockEntities.put(key(x, y, z), d);
                    }
                }
            }
        }
        s.entities.addAll(entities(size));
        return s;
    }

    /** {@code TemplateSource.readStructure}, then placing it as a structure block does, without updates. */
    private static Vec3i placeStructure(CompoundTag root, Vec3i expectedSize) {
        int version = NbtUtils.getDataVersion(root, 500);
        CompoundTag fixed = DataFixTypes.STRUCTURE.updateToCurrentVersion(DataFixers.getDataFixer(), root, version);
        StructureTemplate template = new StructureTemplate();
        template.load(blocks, fixed);
        clear(expectedSize == null ? template.getSize() : max(expectedSize, template.getSize()));
        StructurePlaceSettings settings = new StructurePlaceSettings()
                .setKnownShape(true)
                .setLiquidSettings(LiquidSettings.IGNORE_WATERLOGGING);
        template.placeInWorld(level, ORIGIN, ORIGIN, settings, level.getRandom(), FLAGS);
        // Placing a structure gives a block entity only to blocks that make
        // one; a moving piston's comes from its data, as a chunk loads it.
        for (StructureTemplate.StructureBlockInfo info : template.filterBlocks(ORIGIN, settings, Blocks.MOVING_PISTON)) {
            if (info.nbt() != null && level.getBlockEntity(info.pos()) == null) {
                BlockEntity be = BlockEntity.loadStatic(info.pos(), level.getBlockState(info.pos()), info.nbt(), level.registryAccess());
                if (be != null) {
                    level.setBlockEntity(be);
                }
            }
        }
        return template.getSize();
    }

    /** Sponge version 3 (or 2), as WorldEdit pastes it. */
    private static Vec3i placeSponge(CompoundTag root, Vec3i expectedSize) throws Exception {
        CompoundTag s = root.getCompound("Schematic").orElse(root);
        int version = s.getIntOr("DataVersion", CURRENT);
        Vec3i size = new Vec3i(s.getShortOr("Width", (short) 0) & 0xffff, s.getShortOr("Height", (short) 0) & 0xffff,
                s.getShortOr("Length", (short) 0) & 0xffff);
        CompoundTag holder = s.getIntOr("Version", 2) >= 3 ? s.getCompoundOrEmpty("Blocks") : s;
        CompoundTag palette = holder.getCompoundOrEmpty("Palette");
        TreeMap<Integer, BlockState> states = new TreeMap<>();
        for (String text : palette.keySet()) {
            String fixed = version < CURRENT
                    ? DataFixers.getDataFixer().update(References.FLAT_BLOCK_STATE,
                            new Dynamic<Tag>(NbtOps.INSTANCE, StringTag.valueOf(text)), version, CURRENT)
                            .asString(text)
                    : text;
            states.put(palette.getIntOr(text, -1), BlockStateParser.parseForBlock(blocks, fixed, false).blockState());
        }
        byte[] data = holder.getByteArray(holder == s ? "BlockData" : "Data").orElseThrow();
        clear(expectedSize == null ? size : max(expectedSize, size));
        int i = 0;
        int at = 0;
        while (at < data.length) {
            int v = 0;
            int shift = 0;
            byte b;
            do {
                b = data[at++];
                v |= (b & 0x7f) << shift;
                shift += 7;
            } while ((b & 0x80) != 0);
            int x = i % size.getX();
            int z = (i / size.getX()) % size.getZ();
            int y = i / (size.getX() * size.getZ());
            level.setBlock(ORIGIN.offset(x, y, z), states.get(v), FLAGS);
            i++;
        }
        for (CompoundTag be : holder.getListOrEmpty("BlockEntities").compoundStream().toList()) {
            int[] p = be.getIntArray("Pos").orElseThrow();
            CompoundTag tag = be.getCompound("Data").map(CompoundTag::copy).orElseGet(() -> {
                CompoundTag t = be.copy();
                t.remove("Pos");
                t.remove("Id");
                return t;
            });
            tag.putString("id", be.getStringOr("Id", ""));
            loadBlockEntity(ORIGIN.offset(p[0], p[1], p[2]), tag, version);
        }
        for (CompoundTag e : s.getListOrEmpty("Entities").compoundStream().toList()) {
            CompoundTag tag = e.getCompound("Data").map(CompoundTag::copy).orElseGet(CompoundTag::new);
            tag.putString("id", e.getStringOr("Id", ""));
            ListTag pos = e.getListOrEmpty("Pos");
            addEntity(tag, version, pos.getDoubleOr(0, 0), pos.getDoubleOr(1, 0), pos.getDoubleOr(2, 0));
        }
        return size;
    }

    /** Litematica's format, read as Litematica 26.3 reads it. */
    private static Vec3i placeLitematic(CompoundTag root, Vec3i expectedSize) throws Exception {
        int version = root.getIntOr("MinecraftDataVersion", CURRENT);
        CompoundTag regions = root.getCompoundOrEmpty("Regions");
        if (regions.size() != 1) {
            throw new IllegalStateException("expected one region, found " + regions.size());
        }
        CompoundTag r = regions.getCompoundOrEmpty(regions.keySet().iterator().next());
        int[] pos = xyz(r.getCompoundOrEmpty("Position"));
        int[] sz = xyz(r.getCompoundOrEmpty("Size"));
        Vec3i size = new Vec3i(Math.abs(sz[0]), Math.abs(sz[1]), Math.abs(sz[2]));
        int[] min = new int[3];
        for (int a = 0; a < 3; a++) {
            min[a] = pos[a] + (sz[a] < 0 ? sz[a] + 1 : 0);
        }
        List<BlockState> palette = new ArrayList<>();
        for (CompoundTag entry : r.getListOrEmpty("BlockStatePalette").compoundStream().toList()) {
            CompoundTag tag = entry;
            // LitematicaSchematic.convertBlockStatePalette_to_1_20_5: the
            // game's fixer for older files.
            if (version < CURRENT) {
                tag = (CompoundTag) DataFixers.getDataFixer()
                        .update(References.BLOCK_STATE, new Dynamic<Tag>(NbtOps.INSTANCE, tag.copy()), version, CURRENT)
                        .getValue();
            }
            // DataTypeUtils.readBlockStateFromTag: `id` and `properties`, or
            // `Name` and `Properties` when there is no `id`.
            if (!tag.contains("id")) {
                CompoundTag t = new CompoundTag();
                tag.getString("Name").ifPresent(n -> t.putString("id", n));
                tag.getCompound("Properties").ifPresent(p -> t.put("properties", p));
                tag = t;
            }
            palette.add(NbtUtils.readBlockState(blocks, tag));
        }
        long[] longs = r.getLongArray("BlockStates").orElseThrow();
        int bits = Math.max(2, 32 - Integer.numberOfLeadingZeros(Math.max(palette.size(), 2) - 1));
        long mask = (1L << bits) - 1;
        clear(expectedSize == null ? size : max(expectedSize, size));
        int volume = size.getX() * size.getY() * size.getZ();
        for (int i = 0; i < volume; i++) {
            long start = (long) i * bits;
            int a = (int) (start >> 6);
            int b = (int) (((long) (i + 1) * bits - 1) >> 6);
            int off = (int) (start & 63);
            long v = a == b ? (longs[a] >>> off) & mask : ((longs[a] >>> off) | (longs[b] << (64 - off))) & mask;
            int x = i % size.getX();
            int y = i / (size.getX() * size.getZ());
            int z = (i / size.getX()) % size.getZ();
            level.setBlock(ORIGIN.offset(min[0] + x, min[1] + y, min[2] + z), palette.get((int) v), FLAGS);
        }
        for (CompoundTag te : r.getListOrEmpty("TileEntities").compoundStream().toList()) {
            CompoundTag tag = te.copy();
            BlockPos p = ORIGIN.offset(min[0] + te.getIntOr("x", 0), min[1] + te.getIntOr("y", 0), min[2] + te.getIntOr("z", 0));
            tag.remove("x");
            tag.remove("y");
            tag.remove("z");
            if (!tag.contains("id") && level.getBlockEntity(p) != null) {
                tag.putString("id", BuiltInRegistries.BLOCK_ENTITY_TYPE.getKey(level.getBlockEntity(p).getType()).toString());
            }
            loadBlockEntity(p, tag, version);
        }
        for (CompoundTag e : r.getListOrEmpty("Entities").compoundStream().toList()) {
            ListTag p = e.getListOrEmpty("Pos");
            addEntity(e.copy(), version, pos[0] + p.getDoubleOr(0, 0), pos[1] + p.getDoubleOr(1, 0), pos[2] + p.getDoubleOr(2, 0));
        }
        return size;
    }

    /** Block entity data, fixed from the file's version, loaded into the block entity there. */
    private static void loadBlockEntity(BlockPos p, CompoundTag tag, int version) {
        if (version < CURRENT) {
            tag = (CompoundTag) DataFixers.getDataFixer()
                    .update(References.BLOCK_ENTITY, new Dynamic<Tag>(NbtOps.INSTANCE, tag), version, CURRENT)
                    .getValue();
        }
        BlockEntity be = level.getBlockEntity(p);
        if (be != null) {
            be.loadWithComponents(TagValueInput.create(ProblemReporter.DISCARDING, level.registryAccess(), tag));
            return;
        }
        // A block that makes no block entity of its own, such as a moving
        // piston, gets one from its data, as a chunk loads it.
        be = BlockEntity.loadStatic(p, level.getBlockState(p), tag, level.registryAccess());
        if (be == null) {
            throw new IllegalStateException("the game made no block entity at " + p + " from " + tag);
        }
        level.setBlockEntity(be);
    }

    /** Entity data, fixed from the file's version, loaded and added to the level at a point in the box. */
    private static void addEntity(CompoundTag tag, int version, double x, double y, double z) {
        if (version < CURRENT) {
            tag = (CompoundTag) DataFixers.getDataFixer()
                    .update(References.ENTITY, new Dynamic<Tag>(NbtOps.INSTANCE, tag), version, CURRENT)
                    .getValue();
        }
        Entity e = EntityType.loadEntityRecursive(tag, level, new EntitySpawnRequest(EntitySpawnReason.STRUCTURE, false), en -> {
            en.snapTo(ORIGIN.getX() + x, ORIGIN.getY() + y, ORIGIN.getZ() + z, en.getYRot(), en.getXRot());
            return en;
        });
        if (e != null) {
            level.addFreshEntityWithPassengers(e);
        }
    }

    // --- describing the level -----------------------------------------------

    private static String describe(BlockEntity be) {
        if (be instanceof ComparatorBlockEntity c) {
            return "output " + c.getOutputSignal();
        }
        if (be instanceof PistonMovingBlockEntity m) {
            return "moving " + BlockStateParser.serialize(m.getMovedState());
        }
        if (be instanceof Container c) {
            TreeMap<Integer, String> items = new TreeMap<>();
            for (int i = 0; i < c.getContainerSize(); i++) {
                ItemStack s = c.getItem(i);
                if (!s.isEmpty()) {
                    items.put(i, BuiltInRegistries.ITEM.getKey(s.getItem()) + "*" + s.getCount());
                }
            }
            return items.isEmpty() ? null : "items " + items;
        }
        return null;
    }

    /** Every entity in the box, as type, position and the block state it carries, then removed. */
    private static List<String> entities(Vec3i size) {
        // Entities a little outside the box are part of the file too.
        AABB box = new AABB(ORIGIN.getX(), ORIGIN.getY(), ORIGIN.getZ(), ORIGIN.getX() + size.getX(),
                ORIGIN.getY() + size.getY(), ORIGIN.getZ() + size.getZ()).inflate(2);
        List<String> out = new ArrayList<>();
        for (Entity e : level.getEntitiesOfClass(Entity.class, box, en -> !(en instanceof Player))) {
            StringBuilder s = new StringBuilder(BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString());
            s.append(String.format(" at %s %s %s", e.getX() - ORIGIN.getX(), e.getY() - ORIGIN.getY(), e.getZ() - ORIGIN.getZ()));
            if (e instanceof FallingBlockEntity f) {
                s.append(" ").append(BlockStateParser.serialize(f.getBlockState()));
            }
            if (e instanceof AbstractMinecart m) {
                s.append(" showing ").append(BlockStateParser.serialize(m.getDisplayBlockState()));
            }
            if (e instanceof ItemEntity i) {
                s.append(" ").append(BuiltInRegistries.ITEM.getKey(i.getItem().getItem())).append("*").append(i.getItem().getCount());
            }
            if (e.getVehicle() != null) {
                s.append(" riding ").append(BuiltInRegistries.ENTITY_TYPE.getKey(e.getVehicle().getType()));
            }
            out.add(s.toString());
        }
        for (Entity e : level.getEntitiesOfClass(Entity.class, box.inflate(2), en -> !(en instanceof Player))) {
            e.discard();
        }
        out.sort(Comparator.naturalOrder());
        return out;
    }

    // --- a structure the game saves -----------------------------------------

    /**
     * A small build with block entities and entities that carry block
     * states, saved as {@code StructureTemplate.save} writes it. Entity ids
     * are fixed so the file comes out the same every time.
     */
    private static void saveSample(Path file) throws Exception {
        Vec3i size = new Vec3i(3, 3, 2);
        clear(size);
        set(0, 0, 0, "minecraft:stone");
        set(1, 0, 0, "minecraft:repeater[delay=2,facing=east,locked=false,powered=false]");
        set(2, 0, 0, "minecraft:comparator[facing=west,mode=subtract,powered=true]");
        ((ComparatorBlockEntity) level.getBlockEntity(ORIGIN.offset(2, 0, 0))).setOutputSignal(5);
        set(0, 0, 1, "minecraft:chest[facing=north,type=single,waterlogged=false]");
        ((Container) level.getBlockEntity(ORIGIN.offset(0, 0, 1))).setItem(3, new ItemStack(Items.REDSTONE, 64));
        BlockPos piston = ORIGIN.offset(1, 0, 1);
        BlockState moving = parse("minecraft:moving_piston[facing=up,type=sticky]");
        level.setBlock(piston, moving, FLAGS);
        level.setBlockEntity(MovingPistonBlock.newMovingBlockEntity(piston, moving,
                parse("minecraft:redstone_lamp[lit=true]"), Direction.UP, true, false));
        set(2, 0, 1, "minecraft:gravel");
        int n = 0;
        for (String[] e : new String[][] {
                {"minecraft:falling_block", "{BlockState:{id:\"minecraft:gravel\"},Time:1}", "0.5", "1.5", "0.5"},
                {"minecraft:falling_block", "{BlockState:{id:\"minecraft:anvil\",properties:{facing:\"east\"}},Time:1}", "1.5", "1.5", "0.5"},
                {"minecraft:minecart", "{DisplayState:{id:\"minecraft:redstone_lamp\",properties:{lit:\"true\"}}}", "2.5", "1.0625", "0.5"},
                {"minecraft:falling_block", "{BlockState:{id:\"minecraft:red_sand\"},Time:1}", "0.5", "1.5", "1.5"}}) {
            CompoundTag tag = TagParser.parseCompoundFully(e[1]);
            tag.putString("id", e[0]);
            double x = Double.parseDouble(e[2]);
            double y = Double.parseDouble(e[3]);
            double z = Double.parseDouble(e[4]);
            Entity entity = EntityType.loadEntityRecursive(tag, level, new EntitySpawnRequest(EntitySpawnReason.COMMAND, false), en -> {
                en.snapTo(ORIGIN.getX() + x, ORIGIN.getY() + y, ORIGIN.getZ() + z, 0, 0);
                return en;
            });
            entity.setUUID(new UUID(0x7469636b77726967L, ++n));
            level.addFreshEntityWithPassengers(entity);
        }
        StructureTemplate template = new StructureTemplate();
        template.fillFromWorld(level, ORIGIN, size, true, List.of(Blocks.STRUCTURE_VOID));
        CompoundTag saved = template.save(new CompoundTag());
        Files.createDirectories(file.toAbsolutePath().getParent());
        NbtIo.writeCompressed(saved, file);
        entities(size);
        System.out.printf("saved %s: palette %s%n", file, saved.getListOrEmpty("palette"));
    }

    // --- helpers ------------------------------------------------------------

    private static void set(int x, int y, int z, String state) {
        level.setBlock(ORIGIN.offset(x, y, z), parse(state), FLAGS);
    }

    private static BlockState parse(String s) {
        try {
            return BlockStateParser.parseForBlock(blocks, s, false).blockState();
        } catch (Exception e) {
            throw new IllegalArgumentException("cannot parse block state " + s, e);
        }
    }

    /** Air in the box (and a margin), no block entities, no entities. */
    private static void clear(Vec3i size) {
        for (int y = -1; y <= size.getY(); y++) {
            for (int z = -1; z <= size.getZ(); z++) {
                for (int x = -1; x <= size.getX(); x++) {
                    level.setBlock(ORIGIN.offset(x, y, z), Blocks.AIR.defaultBlockState(), FLAGS | Block.UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS);
                }
            }
        }
        AABB box = new AABB(ORIGIN.getX() - 4, ORIGIN.getY() - 4, ORIGIN.getZ() - 4, ORIGIN.getX() + size.getX() + 4,
                ORIGIN.getY() + size.getY() + 4, ORIGIN.getZ() + size.getZ() + 4);
        for (Entity e : level.getEntitiesOfClass(Entity.class, box, en -> !(en instanceof Player))) {
            e.discard();
        }
    }

    private static Vec3i max(Vec3i a, Vec3i b) {
        return new Vec3i(Math.max(a.getX(), b.getX()), Math.max(a.getY(), b.getY()), Math.max(a.getZ(), b.getZ()));
    }

    /** Load the chunks the boxes fall in with their entities, as the capture does. */
    private static void warmUp(int extent) throws Exception {
        int lo = ORIGIN.getX() - 4;
        int hi = ORIGIN.getX() + extent;
        List<ChunkPos> chunks = new ArrayList<>();
        for (int cx = (lo >> 4) - 1; cx <= (hi >> 4) + 1; cx++) {
            for (int cz = (lo >> 4) - 1; cz <= (hi >> 4) + 1; cz++) {
                ChunkPos c = new ChunkPos(cx, cz);
                if (cx >= lo >> 4 && cx <= hi >> 4 && cz >= lo >> 4 && cz <= hi >> 4) {
                    chunks.add(c);
                }
                level.getChunkSource().addTicketWithRadius(TicketType.PLAYER_SIMULATION, c, 2);
                level.getChunkSource().addTicketWithRadius(TicketType.FORCED, c, 2);
                level.setChunkForced(cx, cz, true);
            }
        }
        long deadline = System.nanoTime() + 300_000_000_000L;
        while (System.nanoTime() < deadline
                && !chunks.stream().allMatch(c -> level.isPositionTickingWithEntitiesLoaded(c.pack()))) {
            tickServer.invoke(server, (BooleanSupplier) () -> true);
            waitUntilNextTick.invoke(server);
            Thread.sleep(5);
        }
        long missing = chunks.stream().filter(c -> !level.isPositionTickingWithEntitiesLoaded(c.pack())).count();
        if (missing > 0) {
            throw new IllegalStateException(missing + " of " + chunks.size() + " chunks never loaded with their entities");
        }
    }

    private static String key(int x, int y, int z) {
        return String.format("%03d %03d %03d", y, z, x);
    }

    private static int[] xyz(CompoundTag t) {
        return new int[] {t.getIntOr("x", 0), t.getIntOr("y", 0), t.getIntOr("z", 0)};
    }

    private static int[] ints(JsonArray a) {
        int[] out = new int[a.size()];
        for (int i = 0; i < out.length; i++) {
            out[i] = a.get(i).getAsInt();
        }
        return out;
    }

    private static String arg(String[] args, String name, String fallback) {
        for (int i = 0; i + 1 < args.length; i++) {
            if (args[i].equals(name)) {
                return args[i + 1];
            }
        }
        return fallback;
    }

    private static void deleteRecursively(Path dir) throws Exception {
        if (!Files.exists(dir)) {
            return;
        }
        try (Stream<Path> s = Files.walk(dir)) {
            for (Path p : s.sorted(Comparator.reverseOrder()).toList()) {
                Files.delete(p);
            }
        }
    }
}
