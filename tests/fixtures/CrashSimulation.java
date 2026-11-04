import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Instant;

/** Controlled process fixture: emits representative errors without exhausting memory or crashing the JVM. */
public class CrashSimulation {
    public static void main(String[] args) throws Exception {
        String scenario = args.length == 0 ? "mixin" : args[0];
        System.out.println("HYDROGEN_SIMULATION_BEGIN " + scenario);
        // Keep the real child alive briefly so the launcher's running state can be observed.
        Thread.sleep(1500);
        if (scenario.equals("normal")) {
            System.out.println("HYDROGEN_SIMULATION_NORMAL_EXIT");
            return;
        }
        String error = switch (scenario) {
            case "heap" -> "java.lang.OutOfMemoryError: Java heap space";
            case "metaspace" -> "java.lang.OutOfMemoryError: Metaspace";
            case "native-memory" -> "There is insufficient memory for the Java Runtime Environment to continue.";
            case "java-version" -> "java.lang.UnsupportedClassVersionError: TestMod has been compiled by a more recent version of the Java Runtime";
            case "graphics" -> "GLFW error 65542: WGL: The driver does not appear to support OpenGL";
            case "dependencies" -> "Incompatible mods found! Mod 'Example' requires a missing dependency.";
            case "mixin" -> "org.spongepowered.asm.mixin.throwables.MixinApplyError: Mixin [example.mixins.json:ExampleMixin] FAILED during APPLY";
            case "library" -> "java.lang.NoClassDefFoundError: example/dependency/Api";
            case "native" -> "A fatal error has been detected by the Java Runtime Environment: EXCEPTION_ACCESS_VIOLATION";
            default -> "A simulated failure without a recognized signature.";
        };
        String content = "---- SIMULATED Minecraft Crash Report ----\nTime: " + Instant.now()
            + "\nScenario: " + scenario + "\n" + error
            + "\n--accessToken SIMULATED_ACCESS_SECRET_123\nrefresh_token=SIMULATED_REFRESH_SECRET_456"
            + "\nAuthorization: Bearer SIMULATED_BEARER_SECRET_789\nWorking directory: " + Path.of("").toAbsolutePath() + "\n";
        Files.createDirectories(Path.of("logs"));
        Files.writeString(Path.of("logs/latest.log"), content, StandardCharsets.UTF_8);
        if (scenario.equals("native")) {
            Files.writeString(Path.of("hs_err_pid" + ProcessHandle.current().pid() + ".log"), content, StandardCharsets.UTF_8);
        } else if (!scenario.equals("unknown")) {
            Files.createDirectories(Path.of("crash-reports"));
            Files.writeString(Path.of("crash-reports/crash-simulated-client.txt"), content, StandardCharsets.UTF_8);
        }
        System.err.println(error);
        System.out.println("HYDROGEN_SIMULATION_EXIT_1 " + scenario);
        System.exit(1);
    }
}
