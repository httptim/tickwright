import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.channels.FileChannel;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;

import net.minecraft.util.RandomSource;

/**
 * The angles newly primed TNT kicks at, and this JVM's {@code Math.sin} and
 * {@code Math.cos} of each, for kick-trig.sh to compare with the engine.
 *
 * <pre>java -cp ... KickTrig &lt;seed&gt; &lt;count&gt; &lt;passes&gt; &lt;out&gt;</pre>
 *
 * Draws {@code count} angles from the game's own random source seeded with
 * {@code seed}, exactly as {@code PrimedTnt}'s constructor does, then works
 * out the sines and cosines {@code passes} times so that later passes run in
 * code the JIT compiled. Every pass must give the same bits. Writes the seed
 * and count, then the angle, sine and cosine of each draw as raw bits, all
 * little-endian.
 */
public final class KickTrig {
    public static void main(String[] args) throws IOException {
        long seed = Long.parseLong(args[0]);
        int count = Integer.parseInt(args[1]);
        int passes = Integer.parseInt(args[2]);
        Path out = Path.of(args[3]);

        RandomSource random = RandomSource.create(seed);
        double[] angles = new double[count];
        for (int i = 0; i < count; i++) {
            // As in PrimedTnt(Level, double, double, double, LivingEntity).
            angles[i] = random.nextDouble() * (float) (Math.PI * 2);
        }

        long[] first = kick(angles);
        long[] last = first;
        int changed = 0;
        for (int p = 1; p < passes; p++) {
            last = kick(angles);
            for (int i = 0; i < last.length; i++) {
                if (last[i] != first[i]) {
                    changed++;
                }
            }
        }
        System.out.printf("%s: %d angles from seed %d, %d passes, %d results differing between passes%n",
                random.getClass().getSimpleName(), count, seed, passes, changed);

        ByteBuffer buf = ByteBuffer.allocate(16 + 24 * count).order(ByteOrder.LITTLE_ENDIAN);
        buf.putLong(seed).putLong(count);
        for (int i = 0; i < count; i++) {
            buf.putLong(Double.doubleToRawLongBits(angles[i]));
            buf.putLong(last[2 * i]);
            buf.putLong(last[2 * i + 1]);
        }
        buf.flip();
        try (FileChannel ch = FileChannel.open(out, StandardOpenOption.CREATE, StandardOpenOption.WRITE,
                StandardOpenOption.TRUNCATE_EXISTING)) {
            while (buf.hasRemaining()) {
                ch.write(buf);
            }
        }
        if (changed != 0) {
            System.exit(1);
        }
    }

    /** The sine and cosine of each angle, as the kick computes them. */
    static long[] kick(double[] angles) {
        long[] out = new long[2 * angles.length];
        for (int i = 0; i < angles.length; i++) {
            double rot = angles[i];
            out[2 * i] = Double.doubleToRawLongBits(Math.sin(rot));
            out[2 * i + 1] = Double.doubleToRawLongBits(Math.cos(rot));
        }
        return out;
    }
}
