package tickwright.harness.agent;

/**
 * Where patched game code calls in. Kept free of game types so the agent
 * compiles and loads without the server jar; the capture driver installs a
 * {@link SetListener} that does the real work.
 */
public final class Hooks {
    /** How many methods the agent patched. Zero means the hook never fires. */
    public static volatile int patchedMethods;

    public static volatile SetListener listener;

    private Hooks() {}

    public interface SetListener {
        /** Called just before {@code chunk} writes {@code state} at {@code pos}. */
        void beforeSet(Object chunk, Object pos, Object state);
    }

    public static void set(Object chunk, Object pos, Object state) {
        SetListener l = listener;
        if (l != null) {
            l.beforeSet(chunk, pos, state);
        }
    }
}
