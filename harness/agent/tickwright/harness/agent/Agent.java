package tickwright.harness.agent;

import java.lang.classfile.ClassFile;
import java.lang.classfile.ClassHierarchyResolver;
import java.lang.classfile.ClassModel;
import java.lang.classfile.ClassTransform;
import java.lang.classfile.CodeBuilder;
import java.lang.classfile.CodeElement;
import java.lang.classfile.CodeTransform;
import java.lang.classfile.MethodModel;
import java.lang.classfile.TypeKind;
import java.lang.constant.ClassDesc;
import java.lang.constant.MethodTypeDesc;
import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.Instrumentation;
import java.lang.reflect.AccessFlag;
import java.security.ProtectionDomain;
import java.util.List;

/**
 * A Java agent that makes the game's internal events observable.
 *
 * <p>Vanilla has no events for "a block changed" or "a neighbor update was
 * delivered", so the agent inserts a call into {@link Hooks} at the start of
 * each method below. Hooks run before the method body, which is exactly where
 * each event belongs in the game's order.
 *
 * <p>Uses the JDK's own class-file API (final since Java 24), so the harness
 * needs nothing beyond the server jar and a JDK.
 */
public final class Agent {
    private static final String BLOCK_POS = "Lnet/minecraft/core/BlockPos;";

    /** Which argument a hook receives. */
    private enum Arg {
        /** The first {@code BlockPos} parameter. */
        FIRST_BLOCK_POS,
        /** The first parameter, whatever its type. */
        FIRST,
        /** The receiver itself, for methods without parameters. */
        SELF,
    }

    private record Rule(String owner, String method, String descriptorPrefix, String kind, Arg arg) {}

    private static final List<Rule> RULES = List.of(
            // The block write itself, with its new state.
            new Rule("net/minecraft/world/level/chunk/LevelChunk", "setBlockState",
                    "(" + BLOCK_POS + "Lnet/minecraft/world/level/block/state/BlockState;", "set", null),
            // A neighbor-changed update arriving at a block.
            new Rule("net/minecraft/world/level/block/state/BlockBehaviour$BlockStateBase",
                    "handleNeighborChanged", "(", "neighbor", Arg.FIRST_BLOCK_POS),
            // A shape update arriving at a block.
            new Rule("net/minecraft/world/level/block/state/BlockBehaviour$BlockStateBase",
                    "updateShape", "(", "shape", Arg.FIRST_BLOCK_POS),
            // A scheduled tick being requested (block or fluid; the driver filters).
            new Rule("net/minecraft/world/ticks/LevelTicks", "schedule", "(", "schedule", Arg.FIRST),
            // A scheduled block tick running.
            new Rule("net/minecraft/server/level/ServerLevel", "tickBlock", "(", "tick", Arg.FIRST_BLOCK_POS),
            // A block event being queued and being run.
            new Rule("net/minecraft/server/level/ServerLevel", "blockEvent", "(" + BLOCK_POS,
                    "block_event_queued", Arg.FIRST_BLOCK_POS),
            new Rule("net/minecraft/server/level/ServerLevel", "doBlockEvent", "(", "block_event", Arg.FIRST),
            // A block entity marked changed: how containers report new contents.
            new Rule("net/minecraft/world/level/block/entity/BlockEntity", "setChanged", "()V", "items", Arg.SELF));

    private Agent() {}

    public static void premain(String args, Instrumentation inst) {
        inst.addTransformer(new ClassFileTransformer() {
            @Override
            public byte[] transform(ClassLoader loader, String name, Class<?> redefined,
                    ProtectionDomain domain, byte[] bytes) {
                if (name.equals("net/minecraft/world/level/levelgen/RandomSupport")) {
                    try {
                        return patchUniqueSeed(loader, bytes);
                    } catch (Throwable t) {
                        System.err.println("tickwright agent: failed to patch " + name);
                        t.printStackTrace();
                        return null;
                    }
                }
                List<Rule> rules = RULES.stream().filter(r -> r.owner().equals(name)).toList();
                if (rules.isEmpty()) {
                    return null;
                }
                try {
                    return patch(loader, bytes, rules);
                } catch (Throwable t) {
                    System.err.println("tickwright agent: failed to patch " + name);
                    t.printStackTrace();
                    return null;
                }
            }
        });
    }

    private static Rule ruleFor(List<Rule> rules, MethodModel m) {
        var flags = m.flags();
        if (flags.has(AccessFlag.STATIC) || flags.has(AccessFlag.BRIDGE)
                || flags.has(AccessFlag.SYNTHETIC) || flags.has(AccessFlag.ABSTRACT)) {
            return null;
        }
        String desc = m.methodTypeSymbol().descriptorString();
        for (Rule r : rules) {
            if (m.methodName().equalsString(r.method()) && desc.startsWith(r.descriptorPrefix())
                    && (r.arg() != Arg.FIRST_BLOCK_POS || desc.contains(BLOCK_POS))
                    && (r.arg() != Arg.FIRST || m.methodTypeSymbol().parameterCount() > 0)) {
                return r;
            }
        }
        return null;
    }

    private static byte[] patch(ClassLoader loader, byte[] bytes, List<Rule> rules) {
        ClassHierarchyResolver resolver = loader == null
                ? ClassHierarchyResolver.defaultResolver()
                : ClassHierarchyResolver.ofResourceParsing(loader)
                        .orElse(ClassHierarchyResolver.defaultResolver());
        ClassFile cf = ClassFile.of(ClassFile.ClassHierarchyResolverOption.of(resolver));
        ClassModel model = cf.parse(bytes);
        ClassDesc hooks = ClassDesc.of("tickwright.harness.agent.Hooks");
        MethodTypeDesc setType = MethodTypeDesc.ofDescriptor(
                "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;)V");
        MethodTypeDesc eventType = MethodTypeDesc.ofDescriptor(
                "(Ljava/lang/String;Ljava/lang/Object;Ljava/lang/Object;)V");

        ClassTransform transform = (cb, element) -> {
            if (element instanceof MethodModel m && ruleFor(rules, m) instanceof Rule rule) {
                MethodTypeDesc type = m.methodTypeSymbol();
                int slot = argSlot(type, rule.arg());
                Hooks.patched.merge(rule.kind(), 1, Integer::sum);
                cb.transformMethod(m, java.lang.classfile.MethodTransform.transformingCode(new CodeTransform() {
                    @Override
                    public void atStart(CodeBuilder b) {
                        if (rule.kind().equals("set")) {
                            b.aload(0).aload(1).aload(2).invokestatic(hooks, "set", setType);
                        } else {
                            b.ldc(rule.kind()).aload(0).aload(slot).invokestatic(hooks, "event", eventType);
                        }
                    }

                    @Override
                    public void accept(CodeBuilder b, CodeElement e) {
                        b.with(e);
                    }
                }));
            } else {
                cb.with(element);
            }
        };
        return cf.transformClass(model, transform);
    }

    /**
     * {@code RandomSupport.generateUniqueSeed()} returns {@code Hooks.uniqueSeed()}
     * unless that is {@code Hooks.NO_SEED}.
     */
    private static byte[] patchUniqueSeed(ClassLoader loader, byte[] bytes) {
        ClassFile cf = ClassFile.of();
        ClassModel model = cf.parse(bytes);
        ClassDesc hooks = ClassDesc.of("tickwright.harness.agent.Hooks");
        ClassTransform transform = (cb, element) -> {
            if (element instanceof MethodModel m && m.methodName().equalsString("generateUniqueSeed")
                    && m.methodTypeSymbol().descriptorString().equals("()J")) {
                Hooks.patched.merge("unique_seed", 1, Integer::sum);
                cb.transformMethod(m, java.lang.classfile.MethodTransform.transformingCode(new CodeTransform() {
                    @Override
                    public void atStart(CodeBuilder b) {
                        var own = b.newLabel();
                        b.invokestatic(hooks, "uniqueSeed", MethodTypeDesc.ofDescriptor("()J"))
                                .dup2()
                                .ldc(Long.MIN_VALUE)
                                .lcmp()
                                .ifeq(own)
                                .lreturn()
                                .labelBinding(own)
                                .pop2();
                    }

                    @Override
                    public void accept(CodeBuilder b, CodeElement e) {
                        b.with(e);
                    }
                }));
            } else {
                cb.with(element);
            }
        };
        return cf.transformClass(model, transform);
    }

    /** The local-variable slot of the argument a rule passes, for an instance method. */
    private static int argSlot(MethodTypeDesc type, Arg arg) {
        if (arg == null) {
            return 1;
        }
        if (arg == Arg.SELF) {
            return 0;
        }
        int slot = 1;
        for (ClassDesc p : type.parameterList()) {
            if (arg == Arg.FIRST || p.descriptorString().equals(BLOCK_POS)) {
                return slot;
            }
            slot += TypeKind.from(p).slotSize();
        }
        throw new IllegalStateException("no matching argument in " + type);
    }
}
