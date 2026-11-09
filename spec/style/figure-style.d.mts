// Generated from spec/style/figure-style.json by scripts/generate_figure_style.py. Do not edit.
export type Mode = "light" | "dark";
export interface ModeColors { surface: string; ink: string; muted: string; grid: string; fill: string; }
export declare const style: Readonly<Record<string, unknown>>;
export declare const colors: Readonly<Record<Mode, ModeColors>>;
export declare const schemeCategorical: Readonly<Record<Mode, readonly string[]>>;
export declare const schemeSequential: Readonly<Record<Mode, readonly string[]>>;
export declare const schemeDiverging: Readonly<Record<Mode, readonly string[]>>;
