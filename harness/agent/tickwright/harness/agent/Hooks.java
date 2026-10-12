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
