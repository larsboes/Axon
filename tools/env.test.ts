import { expect, test } from "bun:test";
import { env } from "./lib/env.ts";

test("the Sjel name wins and the Axon name is the fallback", () => {
  process.env.AXON_ENV_TS_ONLY_OLD = "old";
  process.env.SJEL_ENV_TS_BOTH = "new";
  process.env.AXON_ENV_TS_BOTH = "old";
  expect(env("SJEL_ENV_TS_ONLY_OLD")).toBe("old");
  expect(env("SJEL_ENV_TS_BOTH")).toBe("new");
  expect(env("SJEL_ENV_TS_NEITHER")).toBeUndefined();
  expect(env("HOME")).toBe(process.env.HOME);
});
