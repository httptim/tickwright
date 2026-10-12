package tickwright.harness;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.mojang.authlib.GameProfile;
import java.io.IOException;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Map;
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
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.gametest.framework.GameTestServer;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.TicketType;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.util.Util;
import net.minecraft.world.Container;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.ComparatorBlockEntity;
import net.minecraft.world.level.block.entity.LecternBlockEntity;
import net.minecraft.core.component.DataComponents;
import net.minecraft.server.network.Filterable;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.component.WritableBookContent;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.redstone.CollectingNeighborUpdater;
import net.minecraft.world.level.redstone.NeighborUpdater;
import net.minecraft.world.level.BlockEventData;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.ticks.ScheduledTick;
import net.minecraft.world.level.chunk.LevelChunk;
import net.minecraft.world.level.storage.LevelStorageSource;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;
import tickwright.harness.agent.Hooks;

/**
 * Records ground-truth traces from the real game.
 *
 * <p>Boots a headless {@link GameTestServer} in-process (the approach
 * Nucleation's oracle uses; 26.x jars are unobfuscated, so no mappings or mod
 * loader are needed), then for each resolved fixture: clears the box, places
 * the blocks without updates, applies each stimulus just before its tick, and
 * records every block write inside the box with the tick phase it happened in.
 *
 * <p>Usage: {@code Capture --fixtures <dir> --out <dir> [--universe <dir>] [--only <substring>]}
 */
public final class Capture {
    private static final int QUIET = 2 | 16; // UPDATE_CLIENTS | UPDATE_KNOWN_SHAPE
    private static final int NORMAL = 3; // UPDATE_NEIGHBORS | UPDATE_CLIENTS
    /** Idle ticks after clearing a box, so ticks scheduled by the previous fixture expire. */
    private static final int SETTLE_TICKS = 40;

    private static MinecraftServer server;
    private static Method tickServer;
    private static Method waitUntilNextTick;

    // Recording state, read by the hook.
    private static ServerLevel level;
    private static Recording recording;
    private static boolean inInput;
    private static final List<String> unknownPhases = new ArrayList<>();

    private Capture() {}

    public static void main(String[] args) throws Exception {
        Path fixtures = Path.of(arg(args, "--fixtures", "work/resolved"));
        Path out = Path.of(arg(args, "--out", "work/traces"));
        Path universe = Path.of(arg(args, "--universe", "work/universe"));
        String only = arg(args, "--only", null);
        String physics = arg(args, "--physics", null);

        // LevelChunk is loaded lazily; load it (without initializing) so the
        // agent has patched it before the check below.
        Class.forName("net.minecraft.world.level.chunk.LevelChunk", false, Capture.class.getClassLoader());

        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Util.startTimerHackThread();

        // Load the remaining patched classes so every hook can be checked.
        for (String c : List.of("net.minecraft.world.level.block.state.BlockBehaviour$BlockStateBase",
                "net.minecraft.world.ticks.LevelTicks", "net.minecraft.server.level.ServerLevel",
                "net.minecraft.world.level.block.entity.BlockEntity")) {
            Class.forName(c, false, Capture.class.getClassLoader());
        }
        System.out.printf("agent patched: %s%n", new TreeMap<>(Hooks.patched));
        Class.forName("net.minecraft.util.RandomSupport", false, Capture.class.getClassLoader());
        for (String kind : List.of("set", "neighbor", "shape", "schedule", "tick", "block_event_queued",
                "block_event", "items", "unique_seed")) {
            if (!Hooks.patched.containsKey(kind)) {
                throw new IllegalStateException("the agent patched no method for " + kind
                        + "; run with -javaagent:agent.jar, or the game's method changed");
            }
        }

        deleteRecursively(universe);
        LevelStorageSource storage = LevelStorageSource.createDefault(universe);
        LevelStorageSource.LevelStorageAccess access = storage.createAccess("tickwright");
        PackRepository packs = ServerPacksSource.createPackRepository(access);
        GameTestServer gts = GameTestServer.create(
                Thread.currentThread(), access, packs, Optional.empty(), false, 1);
        server = gts;

        Method initServer = MinecraftServer.class.getDeclaredMethod("initServer");
        tickServer = MinecraftServer.class.getDeclaredMethod("tickServer", BooleanSupplier.class);
        // runServer() also drains the task queue between ticks, which is where
        // chunk entity loading completes. Without it scheduled ticks never run.
        waitUntilNextTick = MinecraftServer.class.getDeclaredMethod("waitUntilNextTick");
        initServer.setAccessible(true);
        tickServer.setAccessible(true);
        waitUntilNextTick.setAccessible(true);
        initServer.invoke(server);
        level = server.overworld();
        String mcVersion = SharedConstants.getCurrentVersion().name();
        System.out.printf("Minecraft %s%n", mcVersion);
        if (physics != null) {
            exportPhysics(Path.of(physics));
        }

        List<Path> files;
        try (Stream<Path> s = Files.list(fixtures)) {
            files = s.filter(p -> p.toString().endsWith(".json"))
                    .filter(p -> only == null || p.getFileName().toString().contains(only))
                    .sorted()
                    .toList();
        }
        if (files.isEmpty()) {
            throw new IllegalStateException("no resolved fixtures in " + fixtures);
        }

        // Bring every chunk any fixture touches to full ticking once, up
        // front. Chunks first asked for between fixtures sometimes never
        // became entity-ticking (a 30-block build reaching x = 32 failed in
        // two of three runs), while the up-front warm-up always has.
        int[] lo = {-32, 90, -32};
        int[] hi = {31, 130, 31};
        for (Path file : files) {
            JsonObject f = JsonParser.parseString(Files.readString(file)).getAsJsonObject();
            int[] o = ints(f.getAsJsonArray("origin"));
            int[] size = ints(f.getAsJsonArray("size"));
            int margin = f.get("margin").getAsInt();
            for (int i = 0; i < 3; i++) {
                lo[i] = Math.min(lo[i], o[i] - margin);
                hi[i] = Math.max(hi[i], o[i] + size[i] - 1 + margin);
            }
        }
        warmUp(new BlockPos(lo[0], lo[1], lo[2]), new BlockPos(hi[0], hi[1], hi[2]));
        strayMinY = lo[1] - 16;
        strayMaxY = hi[1] + 16;

        Hooks.setListener = Capture::beforeSet;
        Hooks.eventListener = Capture::onEvent;
        Files.createDirectories(out);

        Map<String, Integer> phaseCounts = new TreeMap<>();
        Map<String, Integer> kindCounts = new TreeMap<>();
        int failures = 0;
        for (Path file : files) {
            try {
                JsonObject trace = capture(JsonParser.parseString(Files.readString(file)).getAsJsonObject(),
                        mcVersion);
                for (JsonElement e : trace.getAsJsonArray("entries")) {
                    phaseCounts.merge(e.getAsJsonObject().get("phase").getAsString(), 1, Integer::sum);
                    kindCounts.merge(e.getAsJsonObject().get("kind").getAsString(), 1, Integer::sum);
                }
                String id = trace.get("fixture").getAsString();
                Files.writeString(out.resolve(id + ".json"), GSON.toJson(trace) + "\n");
                System.out.printf("  %-40s %4d entries%n", id, trace.getAsJsonArray("entries").size());
            } catch (Exception e) {
                failures++;
                System.out.printf("  %-40s FAILED: %s%n", file.getFileName(), e);
                e.printStackTrace(System.out);
            }
        }

        System.out.printf("entries by phase: %s%n", phaseCounts);
        System.out.printf("entries by kind: %s%n", kindCounts);
        if (!unknownPhases.isEmpty()) {
            failures++;
            System.out.println("block writes in a phase the harness could not identify:");
            unknownPhases.stream().distinct().limit(10).forEach(s -> System.out.println("  " + s));
        }

        server.halt(false);
        deleteRecursively(universe);
        System.out.printf("captured %d of %d fixture(s)%n", files.size() - failures, files.size());
        System.exit(failures == 0 ? 0 : 1);
    }

    private static final Gson GSON = new GsonBuilder().disableHtmlEscaping().create();

    private static final class Recording {
        final BlockPos origin;
        final BlockPos min;
        final BlockPos max;
        final JsonArray entries = new JsonArray();
        final JsonArray entities = new JsonArray();
        final JsonArray rng = new JsonArray();
        /** Entity ids in the order they first appeared, numbered from 0. */
        final Map<Integer, Integer> entityNumbers = new java.util.HashMap<>();
        final java.util.Set<Integer> loggedIds = new java.util.HashSet<>();
        int tick;
        String lastKey;

        Recording(BlockPos origin, BlockPos min, BlockPos max) {
            this.origin = origin;
            this.min = min;
            this.max = max;
        }

        boolean contains(BlockPos p) {
            return p.getX() >= min.getX() && p.getX() <= max.getX()
                    && p.getY() >= min.getY() && p.getY() <= max.getY()
                    && p.getZ() >= min.getZ() && p.getZ() <= max.getZ();
        }
    }

    private static JsonObject capture(JsonObject fixture, String mcVersion) throws Exception {
        int format = fixture.get("format").getAsInt();
        if (format != 1) {
            throw new IllegalArgumentException("resolved fixture format " + format + " is not supported");
        }
        String id = fixture.get("id").getAsString();
        int ticks = fixture.get("ticks").getAsInt();
        int margin = fixture.get("margin").getAsInt();
        BlockPos origin = pos(fixture.getAsJsonArray("origin"));
        int[] size = ints(fixture.getAsJsonArray("size"));
        BlockPos min = origin.offset(-margin, -margin, -margin);
        BlockPos max = origin.offset(size[0] - 1 + margin, size[1] - 1 + margin, size[2] - 1 + margin);

        HolderLookup<Block> blocks = level.registryAccess().lookupOrThrow(Registries.BLOCK);

        warmUp(min, max);
        clear(min, max);
        idle(SETTLE_TICKS);
        clear(min, max);

        setExperiments(id, fixture.has("experiments") ? fixture.getAsJsonArray("experiments") : new JsonArray());
        if (fixture.has("experiments")) {
            // Experimental redstone picks random orientations where nothing
            // gives one; a fixed seed keeps the capture repeatable.
            level.getRandom().setSeed(id.hashCode());
        }
        boolean trackEntities = fixture.has("entities") && fixture.get("entities").getAsBoolean();
        if (trackEntities) {
            // Entities draw from the level's random source when they spawn
            // and when explosions sample rays; a fixed seed keeps the
            // capture repeatable.
            level.getRandom().setSeed(id.hashCode());
            // Entity ids decide when resting items move ((tickCount + id) % 4);
            // number them from 1 in every capture.
            resetEntityIds();
            // Each entity's own random source gets the next seed in a count
            // from 0, in place of one from the clock.
            Hooks.nextSeed = 0;
            Hooks.seedThread = Thread.currentThread();
        }
        setMaxChainedUpdates(fixture.has("max_chained_updates")
                ? fixture.get("max_chained_updates").getAsInt() : -1);
        if (fixture.has("day_time")) {
            freezeClock();
            // Start on a multiple of 20, so tick t runs at game time
            // (start + t) and the daylight detector updates on ticks 20, 40, ...
            while (level.getGameTime() % 20 != 0) {
                tick();
            }
            command("time set " + fixture.get("day_time").getAsLong());
        }

        for (JsonElement b : fixture.getAsJsonArray("blocks")) {
            JsonArray a = b.getAsJsonArray();
            BlockPos p = origin.offset(a.get(0).getAsInt(), a.get(1).getAsInt(), a.get(2).getAsInt());
            level.setBlock(p, parse(blocks, a.get(3).getAsString()), QUIET);
        }
        // Container contents, slot by slot, as the engine fills them.
        if (fixture.has("containers")) {
            for (JsonElement ce : fixture.getAsJsonArray("containers")) {
                JsonObject c = ce.getAsJsonObject();
                BlockPos p = origin.offset(pos(c.getAsJsonArray("pos")));
                if (!(level.getBlockEntity(p) instanceof Container container)) {
                    throw new IllegalStateException(id + ": no container at " + c.get("pos"));
                }
                for (JsonElement ie : c.getAsJsonArray("items")) {
                    JsonObject i = ie.getAsJsonObject();
                    container.setItem(i.get("slot").getAsInt(),
                            new ItemStack(item(i.get("id").getAsString()), i.get("count").getAsInt()));
                }
            }
        }
        // Stored comparator outputs, set after the containers.
        if (fixture.has("comparators")) {
            for (JsonElement ce : fixture.getAsJsonArray("comparators")) {
                JsonObject c = ce.getAsJsonObject();
                BlockPos p = origin.offset(pos(c.getAsJsonArray("pos")));
                if (!(level.getBlockEntity(p) instanceof ComparatorBlockEntity comparator)) {
                    throw new IllegalStateException(id + ": no comparator at " + c.get("pos"));
                }
                comparator.setOutputSignal(c.get("output").getAsInt());
            }
        }

        // Books on lecterns, open at a page.
        if (fixture.has("lecterns")) {
            for (JsonElement le : fixture.getAsJsonArray("lecterns")) {
                JsonObject l = le.getAsJsonObject();
                BlockPos p = origin.offset(pos(l.getAsJsonArray("pos")));
                if (!(level.getBlockEntity(p) instanceof LecternBlockEntity lectern)) {
                    throw new IllegalStateException(id + ": no lectern at " + l.get("pos"));
                }
                List<Filterable<String>> pages = new ArrayList<>();
                for (int i = 0; i < l.get("pages").getAsInt(); i++) {
                    pages.add(Filterable.passThrough("page " + (i + 1)));
                }
                ItemStack book = new ItemStack(Items.WRITABLE_BOOK);
                book.set(DataComponents.WRITABLE_BOOK_CONTENT, new WritableBookContent(pages));
                lectern.setBook(book);
                setPage(lectern, l.has("page") ? l.get("page").getAsInt() : 0);
            }
        }

        JsonArray initial = new JsonArray();
        for (BlockPos p : BlockPos.betweenClosed(min, max)) {
            BlockState s = level.getBlockState(p);
            if (!s.isAir()) {
                JsonArray a = new JsonArray();
                a.add(p.getX() - origin.getX());
                a.add(p.getY() - origin.getY());
                a.add(p.getZ() - origin.getZ());
                a.add(BlockStateParser.serialize(s));
                initial.add(a);
            }
        }

        Map<Integer, List<JsonObject>> stimuli = new TreeMap<>();
        for (JsonElement s : fixture.getAsJsonArray("stimuli")) {
            JsonObject o = s.getAsJsonObject();
            stimuli.computeIfAbsent(o.get("tick").getAsInt(), k -> new ArrayList<>()).add(o);
        }

        Recording rec = new Recording(origin, min, max);
        recording = rec;
        if (trackEntities) {
            noteRng(rec, 0);
        }
        try {
            for (int t = 1; t <= ticks; t++) {
                rec.tick = t;
                rec.lastKey = null;
                inInput = true;
                try {
                    for (JsonObject s : stimuli.getOrDefault(t, List.of())) {
                        apply(s, origin, blocks);
                    }
                } finally {
                    inInput = false;
                }
                tick();
                if (trackEntities) {
                    snapshotEntities(rec, t);
                    noteRng(rec, t);
                }
            }
        } finally {
            recording = null;
            Hooks.seedThread = null;
        }

        // Every kind of event phase 1 needs is recorded, so the trace is
        // complete at every level. Comparator updates are not hooked yet.
        JsonArray levels = new JsonArray();
        levels.add("state");
        levels.add("phase");
        levels.add("order");

        JsonObject trace = new JsonObject();
        trace.addProperty("format", 1);
        trace.addProperty("source", "vanilla");
        trace.addProperty("mc_version", mcVersion);
        trace.addProperty("fixture", id);
        trace.add("origin", fixture.getAsJsonArray("origin"));
        trace.addProperty("ticks", ticks);
        trace.add("levels", levels);
        trace.add("initial", initial);
        trace.add("entries", rec.entries);
        if (trackEntities) {
            trace.add("entities", rec.entities);
            trace.add("rng", rec.rng);
        }
        return trace;
    }

    /** Called by the agent's hook before every block write in a loaded chunk. */
    private static void beforeSet(Object chunkObj, Object posObj, Object stateObj) {
        Recording rec = recording;
        if (!(chunkObj instanceof LevelChunk chunk) || chunk.getLevel() != level) {
            return;
        }
        if (rec == null) {
            noteStray((BlockPos) posObj);
            return;
        }
        BlockPos pos = (BlockPos) posObj;
        if (!rec.contains(pos)) {
            noteStray(pos);
            return;
        }
        BlockState state = (BlockState) stateObj;
        if (chunk.getBlockState(pos) == state) {
            return; // setBlockState returns early without writing.
        }
        String phase = phase();
        String serialized = BlockStateParser.serialize(state);
        int x = pos.getX() - rec.origin.getX();
        int y = pos.getY() - rec.origin.getY();
        int z = pos.getZ() - rec.origin.getZ();
        // An overload that delegates to another reports the same write twice.
        String key = phase + " " + x + "," + y + "," + z + " " + serialized;
        if (key.equals(rec.lastKey)) {
            return;
        }
        rec.lastKey = key;

        add(rec, phase, "set", pos, serialized, null);
    }

    /**
     * Called by the agent's hooks for updates, scheduled ticks and block
     * events. Only the overworld's events inside the box are kept.
     */
    private static void onEvent(String kind, Object self, Object arg) {
        Recording rec = recording;
        if (rec == null) {
            return;
        }
        BlockPos pos;
        String detail = null;
        switch (kind) {
            case "neighbor", "shape" -> pos = (BlockPos) arg;
            case "tick", "block_event_queued" -> {
                if (self != level) {
                    return;
                }
                pos = (BlockPos) arg;
            }
            case "schedule" -> {
                if (self != level.getBlockTicks() || !(arg instanceof ScheduledTick<?> t)) {
                    return;
                }
                pos = t.pos();
                detail = "delay=" + (t.triggerTick() - level.getGameTime())
                        + ",priority=" + t.priority().getValue();
            }
            case "items" -> {
                if (!(self instanceof BlockEntity be) || be.getLevel() != level
                        || !(self instanceof Container container)) {
                    return;
                }
                pos = be.getBlockPos();
                detail = describeItems(container);
            }
            case "block_event" -> {
                if (self != level || !(arg instanceof BlockEventData e)) {
                    return;
                }
                pos = e.pos();
                detail = e.paramA() + "," + e.paramB();
            }
            default -> {
                return;
            }
        }
        if (!rec.contains(pos)) {
            return;
        }
        rec.lastKey = null;
        add(rec, phase(), kind, pos, null, detail);
    }

    private static Item item(String id) {
        for (Item it : BuiltInRegistries.ITEM) {
            if (BuiltInRegistries.ITEM.getKey(it).toString().equals(id)) {
                return it;
            }
        }
        throw new IllegalArgumentException("unknown item " + id);
    }

    /** A container's contents as {@code slot:id*count}, space separated. */
    private static String describeItems(Container container) {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < container.getContainerSize(); i++) {
            ItemStack stack = container.getItem(i);
            if (stack.isEmpty()) {
                continue;
            }
            if (sb.length() > 0) {
                sb.append(' ');
            }
            sb.append(i).append(':').append(BuiltInRegistries.ITEM.getKey(stack.getItem()))
                    .append('*').append(stack.getCount());
        }
        return sb.toString();
    }

    private static void add(Recording rec, String phase, String kind, BlockPos pos, String state, String detail) {
        JsonObject e = new JsonObject();
        e.addProperty("tick", rec.tick);
        e.addProperty("phase", phase);
        e.addProperty("kind", kind);
        JsonArray p = new JsonArray();
        p.add(pos.getX() - rec.origin.getX());
        p.add(pos.getY() - rec.origin.getY());
        p.add(pos.getZ() - rec.origin.getZ());
        e.add("pos", p);
        if (state != null) {
            e.addProperty("state", state);
        }
        if (detail != null) {
            e.addProperty("detail", detail);
        }
        rec.entries.add(e);
    }

    /**
     * The tick phase the current write belongs to, from the innermost game
     * method on the stack that marks a phase. Method names are stable because
     * 26.x jars ship unobfuscated.
     */
    private static String phase() {
        if (inInput) {
            return "input";
        }
        return StackWalker.getInstance().walk(frames -> frames
                .map(f -> phaseOf(f.getClassName(), f.getMethodName()))
                .filter(p -> p != null)
                .findFirst())
                .orElseGet(() -> {
                    StringBuilder sb = new StringBuilder();
                    StackWalker.getInstance().forEach(f -> {
                        if (f.getClassName().startsWith("net.minecraft") && sb.length() < 600) {
                            sb.append(f.getClassName().replace("net.minecraft.", ""))
                                    .append('.').append(f.getMethodName()).append(" < ");
                        }
                    });
                    unknownPhases.add(sb.toString());
                    return "other";
                });
    }

    private static String phaseOf(String cls, String method) {
        if (!cls.startsWith("net.minecraft.")) {
            return null;
        }
        String c = cls.substring(cls.lastIndexOf('.') + 1);
        return switch (c + "." + method) {
            case "ServerLevel.tickBlock" -> "block_ticks";
            case "ServerLevel.tickFluid" -> "fluid_ticks";
            case "ServerLevel.runBlockEvents", "ServerLevel.doBlockEvent" -> "block_events";
            case "Level.tickBlockEntities" -> "block_entities";
            case "ServerLevel.tickNonPassenger", "ServerLevel.tickPassenger" -> "entities";
            case "ServerLevel.tickChunk", "ServerChunkCache.tickChunks" -> "chunks";
            case "Raids.tick" -> "raids";
            default -> null;
        };
    }

    private static void apply(JsonObject s, BlockPos origin, HolderLookup<Block> blocks) {
        BlockPos p = origin.offset(pos(s.getAsJsonArray("pos")));
        switch (s.get("action").getAsString()) {
            case "use" -> useBlock(p);
            case "set" -> level.setBlock(p, parse(blocks, s.get("state").getAsString()), NORMAL);
            case "destroy" -> level.destroyBlock(p, false);
            case "place" -> place(p, s);
            case "page" -> {
                if (!(level.getBlockEntity(p) instanceof LecternBlockEntity lectern)) {
                    throw new IllegalStateException("no lectern at " + p);
                }
                setPage(lectern, s.get("page").getAsInt());
            }
            case "time" -> command("time set " + s.get("time").getAsLong());
            case "summon" -> summon(s, origin);
            default -> throw new IllegalArgumentException("unknown action " + s.get("action"));
        }
    }

    /** An exact decimal for a double, which the command parser reads back to the same double. */
    private static String exact(double v) {
        return new java.math.BigDecimal(v).toPlainString();
    }

    /** {@code /summon} at an exact point, with an exact starting velocity. */
    private static void summon(JsonObject s, BlockPos origin) {
        JsonArray at = s.getAsJsonArray("at");
        String x = exact(origin.getX() + at.get(0).getAsDouble());
        String y = exact(origin.getY() + at.get(1).getAsDouble());
        String z = exact(origin.getZ() + at.get(2).getAsDouble());
        List<String> parts = new ArrayList<>();
        if (s.has("nbt")) {
            String nbt = s.get("nbt").getAsString().strip();
            String inner = nbt.substring(1, nbt.length() - 1).strip();
            if (!inner.isEmpty()) {
                parts.add(inner);
            }
        }
        if (s.has("motion")) {
            JsonArray m = s.getAsJsonArray("motion");
            parts.add("Motion:[" + exact(m.get(0).getAsDouble()) + "d," + exact(m.get(1).getAsDouble()) + "d,"
                    + exact(m.get(2).getAsDouble()) + "d]");
        }
        String cmd = "summon " + s.get("entity").getAsString() + " " + x + " " + y + " " + z
                + (parts.isEmpty() ? "" : " {" + String.join(",", parts) + "}");
        int before = entityCount();
        command(cmd);
        int after = entityCount();
        if (after <= before) {
            throw new IllegalStateException("summon made no entity: " + cmd);
        }
    }

    private static int entityCount() {
        int n = 0;
        for (Entity ignored : level.getAllEntities()) {
            n++;
        }
        return n;
    }

    /** Every entity in the box at the end of tick {@code t}, in order of first appearance. */
    private static void snapshotEntities(Recording rec, int t) {
        // Say where every new entity in the level came from, in or out of the box.
        for (Entity e : level.getAllEntities()) {
            if (!(e instanceof Player) && rec.loggedIds.add(e.getId())) {
                System.out.printf("    %s tick %d: entity %d %s at %s, seeds used %d%n", rec.origin, t, e.getId(),
                        BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()), e.position(), Hooks.nextSeed);
            }
        }
        AABB box = new AABB(rec.min.getX(), rec.min.getY(), rec.min.getZ(),
                rec.max.getX() + 1, rec.max.getY() + 1, rec.max.getZ() + 1);
        List<Entity> found = new ArrayList<>(level.getEntitiesOfClass(Entity.class, box));
        found.sort(Comparator.comparingInt(Entity::getId));
        for (Entity e : found) {
            if (e instanceof Player) {
                continue;
            }
            int n = rec.entityNumbers.computeIfAbsent(e.getId(), k -> rec.entityNumbers.size());
            JsonObject o = new JsonObject();
            o.addProperty("tick", t);
            o.addProperty("n", n);
            o.addProperty("id", e.getId());
            o.addProperty("type", BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString());
            JsonArray pos = new JsonArray();
            pos.add(e.getX() - rec.origin.getX());
            pos.add(e.getY() - rec.origin.getY());
            pos.add(e.getZ() - rec.origin.getZ());
            o.add("pos", pos);
            Vec3 v = e.getDeltaMovement();
            JsonArray motion = new JsonArray();
            motion.add(v.x);
            motion.add(v.y);
            motion.add(v.z);
            o.add("motion", motion);
            String detail = entityDetail(e);
            if (detail != null) {
                o.addProperty("detail", detail);
            }
            rec.entities.add(o);
        }
    }

    private static String entityDetail(Entity e) {
        if (e instanceof net.minecraft.world.entity.item.ItemEntity item) {
            ItemStack stack = item.getItem();
            return BuiltInRegistries.ITEM.getKey(stack.getItem()) + "*" + stack.getCount();
        }
        if (e instanceof net.minecraft.world.entity.item.PrimedTnt tnt) {
            return "fuse=" + tnt.getFuse();
        }
        if (e instanceof net.minecraft.world.entity.item.FallingBlockEntity falling) {
            return BlockStateParser.serialize(falling.getBlockState());
        }
        return null;
    }

    private static java.lang.reflect.Field entityCounter;

    /** Sets {@code ServerLevel.ENTITY_COUNTER} to 0, so the next entity made gets id 1. */
    private static void resetEntityIds() {
        try {
            if (entityCounter == null) {
                // 26.3 keeps it on ServerLevel; older versions on Entity.
                for (Class<?> c : List.of(ServerLevel.class, Entity.class)) {
                    for (java.lang.reflect.Field f : c.getDeclaredFields()) {
                        if (entityCounter == null && java.lang.reflect.Modifier.isStatic(f.getModifiers())
                                && f.getType() == java.util.concurrent.atomic.AtomicInteger.class
                                && f.getName().equals("ENTITY_COUNTER")) {
                            f.setAccessible(true);
                            entityCounter = f;
                        }
                    }
                }
                if (entityCounter == null) {
                    throw new IllegalStateException("no ENTITY_COUNTER on ServerLevel or Entity");
                }
            }
            ((java.util.concurrent.atomic.AtomicInteger) entityCounter.get(null)).set(0);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException("cannot reset entity ids", e);
        }
    }

    /**
     * What entity physics needs from every block: each state's collision
     * boxes (with no world around it, so context-dependent shapes like
     * scaffolding are their defaults), and each block's friction, speed and
     * jump factors and explosion resistance.
     */
    private static void exportPhysics(Path file) throws IOException {
        List<String> shapes = new ArrayList<>();
        Map<String, Integer> shapeIndex = new java.util.HashMap<>();
        JsonArray runs = new JsonArray();
        int lastShape = -1;
        int runStart = 0;
        int id = 0;
        for (;; id++) {
            BlockState state = Block.BLOCK_STATE_REGISTRY.byId(id);
            if (state == null) {
                break;
            }
            JsonArray boxes = new JsonArray();
            for (AABB b : state.getCollisionShape(net.minecraft.world.level.EmptyBlockGetter.INSTANCE, BlockPos.ZERO)
                    .toAabbs()) {
                JsonArray a = new JsonArray();
                a.add(b.minX);
                a.add(b.minY);
                a.add(b.minZ);
                a.add(b.maxX);
                a.add(b.maxY);
                a.add(b.maxZ);
                boxes.add(a);
            }
            String key = GSON.toJson(boxes);
            int index = shapeIndex.computeIfAbsent(key, k -> {
                shapes.add(k);
                return shapes.size() - 1;
            });
            if (index != lastShape) {
                if (lastShape >= 0) {
                    JsonArray r = new JsonArray();
                    r.add(runStart);
                    r.add(lastShape);
                    runs.add(r);
                }
                lastShape = index;
                runStart = id;
            }
        }
        JsonArray last = new JsonArray();
        last.add(runStart);
        last.add(lastShape);
        runs.add(last);

        StringBuilder out = new StringBuilder("{\n");
        out.append("  \"states\": ").append(id).append(",\n");
        out.append("  \"shapes\": [\n    ").append(String.join(",\n    ", shapes)).append("\n  ],\n");
        out.append("  \"shape_runs\": ").append(GSON.toJson(runs)).append(",\n");
        out.append("  \"blocks\": {\n");
        List<String> rows = new ArrayList<>();
        for (Block b : BuiltInRegistries.BLOCK) {
            rows.add("    \"" + BuiltInRegistries.BLOCK.getKey(b) + "\": {\"friction\": " + b.getFriction()
                    + ", \"speed\": " + b.getSpeedFactor() + ", \"jump\": " + b.getJumpFactor()
                    + ", \"resistance\": " + b.getExplosionResistance() + "}");
        }
        out.append(String.join(",\n", rows)).append("\n  }\n}\n");
        Files.writeString(file, out.toString());
        System.out.printf("block physics: %d states, %d distinct shapes -> %s%n", id, shapes.size(), file);
    }

    private static java.lang.reflect.Field rngSeed;

    /** The level random source's internal seed, after tick {@code t}. */
    private static void noteRng(Recording rec, int t) {
        Object random = level.getRandom();
        try {
            if (rngSeed == null) {
                for (Class<?> c = random.getClass(); c != null && rngSeed == null; c = c.getSuperclass()) {
                    for (java.lang.reflect.Field f : c.getDeclaredFields()) {
                        if (f.getType() == java.util.concurrent.atomic.AtomicLong.class) {
                            f.setAccessible(true);
                            rngSeed = f;
                            break;
                        }
                    }
                }
                if (rngSeed == null) {
                    throw new IllegalStateException("no seed field in " + random.getClass());
                }
            }
            JsonArray a = new JsonArray();
            a.add(t);
            a.add(((java.util.concurrent.atomic.AtomicLong) rngSeed.get(random)).get());
            rec.rng.add(a);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException("cannot read the level's random source", e);
        }
    }

    /** {@code LecternBlockEntity.setPage}, which is private: what turning a page in the book menu calls. */
    private static void setPage(LecternBlockEntity lectern, int page) {
        try {
            Method m = LecternBlockEntity.class.getDeclaredMethod("setPage", int.class);
            m.setAccessible(true);
            m.invoke(lectern, page);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException("cannot turn the lectern's page", e);
        }
    }

    /** Run a command as the server console. */
    private static void command(String command) {
        server.getCommands().performPrefixedCommand(server.createCommandSourceStack().withSuppressedOutput(), command);
    }

    private static boolean clockFrozen;

    /** Stop the day-night cycle and the weather, once, for fixtures that read the time of day. */
    private static void freezeClock() {
        if (clockFrozen) {
            return;
        }
        clockFrozen = true;
        // The game rule's name changed in 1.21.11; the other one is rejected quietly.
        command("gamerule advance_time false");
        command("gamerule doDaylightCycle false");
        command("weather clear");
    }

    /** The features the world was created with, which the captures without experiments use. */
    private static FeatureFlagSet baseFlags;
    private static final java.util.Set<String> experimentsOn = new java.util.TreeSet<>();

    /**
     * Turn feature flags on or off for the next fixture. {@code ServerLevel.enabledFeatures}
     * reads the world data each time, so switching it between fixtures is enough.
     */
    private static void setExperiments(String id, JsonArray experiments) {
        java.util.Set<String> wanted = new java.util.TreeSet<>();
        experiments.forEach(e -> wanted.add(e.getAsString()));
        if (wanted.equals(experimentsOn)) {
            return;
        }
        WorldDataConfiguration old = server.getWorldData().getDataConfiguration();
        if (baseFlags == null) {
            baseFlags = old.enabledFeatures();
        }
        FeatureFlagSet flags = baseFlags;
        for (String name : wanted) {
            if (!name.equals("redstone_experiments")) {
                throw new IllegalArgumentException(id + ": unknown experiment " + name);
            }
            flags = flags.join(FeatureFlagSet.of(FeatureFlags.REDSTONE_EXPERIMENTS));
        }
        server.getWorldData().setDataConfiguration(new WorldDataConfiguration(old.dataPacks(), flags));
        boolean on = level.enabledFeatures().contains(FeatureFlags.REDSTONE_EXPERIMENTS);
        if (on != wanted.contains("redstone_experiments")) {
            throw new IllegalStateException(id + ": redstone_experiments did not switch " + (on ? "off" : "on"));
        }
        experimentsOn.clear();
        experimentsOn.addAll(wanted);
        System.out.printf("experiments now %s%n", experimentsOn);
    }

    private static NeighborUpdater defaultUpdater;

    /**
     * Swap the level's neighbor updater for one with a different
     * {@code max-chained-neighbor-updates}, or back to the server's own when
     * {@code limit} is negative.
     */
    private static void setMaxChainedUpdates(int limit) {
        if (limit < 0 && defaultUpdater == null) {
            return; // never changed
        }
        try {
            java.lang.reflect.Field f = null;
            for (Class<?> c = level.getClass(); c != null && f == null; c = c.getSuperclass()) {
                for (java.lang.reflect.Field field : c.getDeclaredFields()) {
                    if (!java.lang.reflect.Modifier.isStatic(field.getModifiers())
                            && NeighborUpdater.class.isAssignableFrom(field.getType())) {
                        f = field;
                    }
                }
            }
            if (f == null) {
                throw new IllegalStateException("the level has no NeighborUpdater field");
            }
            f.setAccessible(true);
            if (defaultUpdater == null) {
                defaultUpdater = (NeighborUpdater) f.get(level);
            }
            f.set(level, limit < 0 ? defaultUpdater : new CollectingNeighborUpdater(level, limit));
            System.out.printf("neighbor updater now %s (limit %d)%n", f.get(level).getClass().getSimpleName(), limit);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException("cannot set max-chained-neighbor-updates", e);
        }
    }

    private static Direction direction(String name) {
        for (Direction d : Direction.values()) {
            if (d.getName().equals(name)) {
                return d;
            }
        }
        throw new IllegalArgumentException("unknown direction " + name);
    }

    /** The yaw of a player facing {@code look}, as the engine assumes it. */
    private static float yaw(String look) {
        return switch (look) {
            case "south" -> 0f;
            case "west" -> 90f;
            case "north" -> 180f;
            case "east" -> 270f;
            default -> throw new IllegalArgumentException("unknown look " + look);
        };
    }

    /**
     * Right-click a face of {@code clicked} holding an item, as a player
     * would: {@code ItemStack.useOn}, which for a block item runs
     * {@code BlockItem.place} and so the block's own placement rules.
     */
    private static void place(BlockPos clicked, JsonObject s) {
        Player player = new Player(level, new GameProfile(UUID.randomUUID(), "tickwright")) {
            @Override
            public GameType gameMode() {
                return GameType.CREATIVE;
            }

            @Override
            public boolean isClientAuthoritative() {
                return false;
            }
        };
        // Far above the build, so the player never obstructs a placement.
        player.setPos(clicked.getX() + 0.5, clicked.getY() + 64, clicked.getZ() + 0.5);
        // Placement reads both: the body's yaw for the horizontal direction,
        // the head's (getViewYRot) for the nearest looking direction.
        player.setYRot(yaw(s.get("look").getAsString()));
        player.setYHeadRot(yaw(s.get("look").getAsString()));
        player.setXRot(s.has("pitch") ? s.get("pitch").getAsFloat() : 0f);
        player.setShiftKeyDown(s.has("sneak") && s.get("sneak").getAsBoolean());
        String id = s.get("item").getAsString();
        ItemStack stack = new ItemStack(item(id.contains(":") ? id : "minecraft:" + id));
        InteractionHand hand = InteractionHand.MAIN_HAND;
        player.setItemInHand(hand, stack);
        Direction face = direction(s.get("face").getAsString());
        // Resolved fixtures always give the click point, in sixteenths.
        JsonArray h = s.getAsJsonArray("hit");
        int[] hit = {h.get(0).getAsInt(), h.get(1).getAsInt(), h.get(2).getAsInt()};
        Vec3 at = new Vec3(clicked.getX() + hit[0] / 16.0, clicked.getY() + hit[1] / 16.0, clicked.getZ() + hit[2] / 16.0);
        BlockHitResult result = new BlockHitResult(at, face, clicked, false);
        stack.useOn(new net.minecraft.world.item.context.UseOnContext(player, hand, result));
    }

    /**
     * Right-click {@code pos} with an empty main hand, as
     * {@code GameTestHelper.useBlock} does. Adapted from Nucleation's
     * TraceCapture (MIT).
     */
    private static void useBlock(BlockPos pos) {
        Player player = new Player(level, new GameProfile(UUID.randomUUID(), "tickwright")) {
            @Override
            public GameType gameMode() {
                return GameType.CREATIVE;
            }

            @Override
            public boolean isClientAuthoritative() {
                return false;
            }
        };
        BlockState state = level.getBlockState(pos);
        InteractionHand hand = InteractionHand.MAIN_HAND;
        BlockHitResult hit = new BlockHitResult(Vec3.atCenterOf(pos), Direction.UP, pos, false);
        InteractionResult result = state.useItemOn(player.getItemInHand(hand), level, player, hand, hit);
        if (result.consumesAction()) {
            return;
        }
        if (result instanceof InteractionResult.TryEmptyHandInteraction) {
            state.useWithoutItem(level, player, hit);
        }
    }

    private static final java.util.Set<Long> warm = new java.util.HashSet<>();

    /**
     * Make sure every chunk the box touches is fully ticking. A chunk that is
     * merely loaded keeps its scheduled ticks queued until its entities finish
     * loading, which looks like redstone that ignores its input.
     */
    private static void warmUp(BlockPos min, BlockPos max) throws Exception {
        List<ChunkPos> chunks = new ArrayList<>();
        for (int cx = (min.getX() >> 4) - 1; cx <= (max.getX() >> 4) + 1; cx++) {
            for (int cz = (min.getZ() >> 4) - 1; cz <= (max.getZ() >> 4) + 1; cz++) {
                ChunkPos c = new ChunkPos(cx, cz);
                boolean covered = cx >= (min.getX() >> 4) && cx <= (max.getX() >> 4)
                        && cz >= (min.getZ() >> 4) && cz <= (max.getZ() >> 4);
                if (covered) {
                    chunks.add(c);
                }
                if (warm.add(c.pack())) {
                    level.getChunkSource().addTicketWithRadius(TicketType.PLAYER_SIMULATION, c, 2);
                    level.getChunkSource().addTicketWithRadius(TicketType.FORCED, c, 2);
                    level.setChunkForced(cx, cz, true);
                }
            }
        }
        // Chunk loading happens on worker threads, so wait on the clock rather
        // than a tick count: unthrottled ticks can outrun it.
        long deadline = System.nanoTime() + 120_000_000_000L;
        while (System.nanoTime() < deadline
                && !chunks.stream().allMatch(c -> level.isPositionTickingWithEntitiesLoaded(c.pack()))) {
            tick();
            Thread.sleep(5);
        }
        if (!chunks.stream().allMatch(c -> level.isPositionTickingWithEntitiesLoaded(c.pack()))) {
            throw new IllegalStateException("chunks around the fixture never became entity-ticking");
        }
    }

    /**
     * Blocks written outside the box being recorded: a flying machine that
     * left its box keeps flying through later captures unless it is removed.
     * Only the height band fixtures use is tracked, away from the terrain.
     */
    private static final java.util.Set<BlockPos> strays = new java.util.HashSet<>();
    private static boolean clearing;
    // Nothing is tracked until the fixtures' height band is known.
    private static int strayMinY = Integer.MAX_VALUE;
    private static int strayMaxY = Integer.MIN_VALUE;

    private static void noteStray(BlockPos pos) {
        if (!clearing && pos.getY() >= strayMinY && pos.getY() <= strayMaxY) {
            strays.add(pos.immutable());
        }
    }

    private static void clear(BlockPos min, BlockPos max) {
        clearing = true;
        for (BlockPos p : strays) {
            level.setBlock(p, Blocks.AIR.defaultBlockState(), QUIET);
            level.removeBlockEntity(p);
        }
        strays.clear();
        for (BlockPos p : BlockPos.betweenClosed(min, max)) {
            level.setBlock(p, Blocks.AIR.defaultBlockState(), QUIET);
        }
        clearing = false;
        AABB box = new AABB(min.getX() - 1, min.getY() - 1, min.getZ() - 1,
                max.getX() + 2, max.getY() + 2, max.getZ() + 2);
        for (Entity e : level.getEntitiesOfClass(Entity.class, box)) {
            e.discard();
        }
    }

    private static void idle(int n) throws Exception {
        for (int i = 0; i < n; i++) {
            tick();
        }
    }

    private static void tick() throws Exception {
        tickServer.invoke(server, (BooleanSupplier) () -> true);
        waitUntilNextTick.invoke(server);
    }

    private static BlockState parse(HolderLookup<Block> blocks, String s) {
        try {
            return BlockStateParser.parseForBlock(blocks, s, false).blockState();
        } catch (Exception e) {
            throw new IllegalArgumentException("cannot parse block state " + s + ": " + e.getMessage(), e);
        }
    }

    private static BlockPos pos(JsonArray a) {
        return new BlockPos(a.get(0).getAsInt(), a.get(1).getAsInt(), a.get(2).getAsInt());
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

    private static void deleteRecursively(Path dir) throws IOException {
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
