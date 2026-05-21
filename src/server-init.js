import initializeApp from "./shared/services/initializeApp.js";
import { initCrashLogger } from "./lib/crashLogger.js";

// Initialize global crash handlers as early as possible.
// This is the #1 fix for silent deaths on Ubuntu (uncaught errors from child processes, OOM pressure, etc.).
// Crashes will now be written to ~/.9router/crash.log with full context.
initCrashLogger();

async function startServer() {
  console.log("Starting server...");
  
  try {
    await initializeApp();
    console.log("Server initialized");
  } catch (error) {
    console.log("Error initializing server:", error);
    process.exit(1);
  }
}

startServer().catch(console.log);

export default startServer;
