import React from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { App } from "./App";
import { registerServiceWorker } from "./lib/pwa";
import "./styles/globals.css";

const client = new QueryClient({
  defaultOptions: {
    queries: {
      // Config is edited from three places — this UI, voice, and the
      // API — so a stale view is actively misleading. Refetch on focus
      // rather than trusting a cache.
      refetchOnWindowFocus: true,
      staleTime: 2_000,
      retry: 1,
    },
  },
});

registerServiceWorker();

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={client}>
      <App />
    </QueryClientProvider>
  </React.StrictMode>,
);
