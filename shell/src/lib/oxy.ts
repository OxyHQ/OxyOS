import { OXY_API_URL, OxyServices } from "@oxy.so/core";

/** The shell's one Oxy client: its session is the bearer every linked product client sends. */
export const oxy = new OxyServices({ baseURL: OXY_API_URL });
