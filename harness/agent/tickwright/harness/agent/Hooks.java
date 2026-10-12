package tickwright.harness.agent;

import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Where patched game code calls in. Kept free of game types so the agent
 * compiles and loads without the server jar; the capture driver installs the
 * listeners that do the real work.
 */
public final class Hooks {
    /** How many methods the agent patched, by event kind. */
    public static final Map<String, Integer> patched = new ConcurrentHashMap<>();

    public static volatile SetListener setListener;
    public static volatile EventListener eventListener;

    private Hooks() {}

    public interface SetListener {
        /** Called just before {@code chunk} writes {@code state} at {@code pos}. */
        void beforeSet(Object chunk, Object pos, Object state);
    }

    public interface EventListener {
        /**
         * Called at the start of a patched method. {@code self} is the
         * receiver; {@code arg} is the argument the agent's rule selected.
         */
        void event(String kind, Object self, Object arg);
    }

    /** Returned by {@link #uniqueSeed} to let the game pick its own seed. */
    public static final long NO_SEED = Long.MIN_VALUE;

    /** While set, {@code RandomSupport.generateUniqueSeed} on this thread counts up from {@link #nextSeed}. */
    public static volatile Thread seedThread;
    public static long nextSeed;

    /**
     * Called at the start of {@code RandomSupport.generateUniqueSeed}: every
     * entity's own random source is created from one, so captures replace
     * the time-based seed with a count to make entities repeatable.
     */
    public static long uniqueSeed() {
        if (Thread.currentThread() == seedThread) {
            return nextSeed++;
        }
        return NO_SEED;
    }

    public static void set(Object chunk, Object pos, Object state) {
        SetListener l = setListener;
        if (l != null) {
            l.beforeSet(chunk, pos, state);
        }
    }

    public static void event(String kind, Object self, Object arg) {
        EventListener l = eventListener;
        if (l != null) {
            l.event(kind, self, arg);
        }
    }
}
