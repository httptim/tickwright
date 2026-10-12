package tickwright.harness.agent;

import java.lang.classfile.ClassFile;
import java.lang.classfile.ClassHierarchyResolver;
import java.lang.classfile.ClassModel;
import java.lang.classfile.ClassTransform;
import java.lang.classfile.CodeBuilder;
import java.lang.classfile.CodeElement;
import java.lang.classfile.CodeTransform;
import java.lang.classfile.MethodModel;
import java.lang.constant.ClassDesc;
import java.lang.constant.MethodTypeDesc;
import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.Instrumentation;
import java.lang.reflect.AccessFlag;
import java.security.ProtectionDomain;

/**
 * A Java agent that makes every block write in the game observable.
 *
 * <p>Vanilla has no event for "a block changed". Every change to a loaded
 * chunk goes through {@code LevelChunk.setBlockState}, so the agent inserts a
 * call to {@link Hooks#set} at the start of that method. The call happens
 * before the write and before any neighbor updates the write causes, which is
 * exactly where the change belongs in the game's order of events.
 *
 * <p>Uses the JDK's own class-file API (final since Java 24), so the harness
 * needs nothing beyond the server jar and a JDK.
 */
public final class Agent {
    private static final String TARGET = "net/minecraft/world/level/chunk/LevelChunk";
    private static final String SET_PREFIX =
            "(Lnet/minecraft/core/BlockPos;Lnet/minecraft/world/level/block/state/BlockState;";

    private Agent() {}

    public static void premain(String args, Instrumentation inst) {
        inst.addTransformer(new ClassFileTransformer() {
            @Override
            public byte[] transform(ClassLoader loader, String name, Class<?> redefined,
                    ProtectionDomain domain, byte[] bytes) {
                if (!TARGET.equals(name)) {
                    return null;
                }
                try {
                    return patch(loader, bytes);
                } catch (Throwable t) {
                    System.err.println("tickwright agent: failed to patch " + name);
                    t.printStackTrace();
                    return null;
                }
            }
        });
    }

    private static boolean isSetBlockState(MethodModel m) {
        var flags = m.flags();
        return m.methodName().equalsString("setBlockState")
                && m.methodTypeSymbol().descriptorString().startsWith(SET_PREFIX)
                && !flags.has(AccessFlag.STATIC)
                && !flags.has(AccessFlag.BRIDGE)
                && !flags.has(AccessFlag.SYNTHETIC)
                && !flags.has(AccessFlag.ABSTRACT);
    }

    private static byte[] patch(ClassLoader loader, byte[] bytes) {
        ClassHierarchyResolver resolver = loader == null
                ? ClassHierarchyResolver.defaultResolver()
                : ClassHierarchyResolver.ofResourceParsing(loader)
                        .orElse(ClassHierarchyResolver.defaultResolver());
        ClassFile cf = ClassFile.of(ClassFile.ClassHierarchyResolverOption.of(resolver));
        ClassModel model = cf.parse(bytes);
        ClassDesc hooks = ClassDesc.of("tickwright.harness.agent.Hooks");
        MethodTypeDesc hookType = MethodTypeDesc.ofDescriptor(
                "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;)V");
        CodeTransform atStart = new CodeTransform() {
            @Override
            public void atStart(CodeBuilder b) {
                Hooks.patchedMethods++;
                b.aload(0).aload(1).aload(2).invokestatic(hooks, "set", hookType);
            }

            @Override
            public void accept(CodeBuilder b, CodeElement e) {
                b.with(e);
            }
        };
        return cf.transformClass(model,
                ClassTransform.transformingMethodBodies(Agent::isSetBlockState, atStart));
    }
}
