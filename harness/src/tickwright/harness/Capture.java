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
import net.minecraft.core.registries.Registries;
import net.minecraft.gametest.framework.GameTestServer;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.TicketType;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.util.Util;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
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

        // LevelChunk is loaded lazily; load it (without initializing) so the
        // agent has patched it before the check below.
        Class.forName("net.minecraft.world.level.chunk.LevelChunk", false, Capture.class.getClassLoader());

        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Util.startTimerHackThread();

        // Load the remaining patched classes so every hook can be checked.
        for (String c : List.of("net.minecraft.world.level.block.state.BlockBehaviour$BlockStateBase",
                "net.minecraft.world.ticks.LevelTicks", "net.minecraft.server.level.ServerLevel")) {
            Class.forName(c, false, Capture.class.getClassLoader());
        }
        System.out.printf("agent patched: %s%n", new TreeMap<>(Hooks.patched));
        for (String kind : List.of("set", "neighbor", "shape", "schedule", "tick", "block_event_queued",
                "block_event")) {
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

        // Bring every chunk near the default origin to full ticking once, up
        // front, so the first fixture in a new chunk does not wait on loading.
        warmUp(new BlockPos(-32, 90, -32), new BlockPos(31, 130, 31));

        Hooks.setListener = Capture::beforeSet;
        Hooks.eventListener = Capture::onEvent;

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

        for (JsonElement b : fixture.getAsJsonArray("blocks")) {
            JsonArray a = b.getAsJsonArray();
            BlockPos p = origin.offset(a.get(0).getAsInt(), a.get(1).getAsInt(), a.get(2).getAsInt());
            level.setBlock(p, parse(blocks, a.get(3).getAsString()), QUIET);
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
            }
        } finally {
            recording = null;
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
        return trace;
    }

    /** Called by the agent's hook before every block write in a loaded chunk. */
    private static void beforeSet(Object chunkObj, Object posObj, Object stateObj) {
        Recording rec = recording;
        if (rec == null || !(chunkObj instanceof LevelChunk chunk) || chunk.getLevel() != level) {
            return;
        }
        BlockPos pos = (BlockPos) posObj;
        if (!rec.contains(pos)) {
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
            default -> throw new IllegalArgumentException("unknown action " + s.get("action"));
        }
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

    private static void clear(BlockPos min, BlockPos max) {
        for (BlockPos p : BlockPos.betweenClosed(min, max)) {
            level.setBlock(p, Blocks.AIR.defaultBlockState(), QUIET);
        }
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
