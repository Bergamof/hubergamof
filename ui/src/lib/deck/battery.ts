/** What the manager says of a battery: the mouse and the headphones say it the same way. */
export interface BatteryReading {
	/** Charge, from 0 to 100. */
	percent: number;
	/** Plugged in and charging. */
	charging: boolean;
}

/** Below this charge (in percent) a battery warns, unless it is charging. */
export const LOW_BATTERY_PERCENT = 20;

/** Whether the battery warns. */
export function isLow(battery: BatteryReading): boolean {
	return !battery.charging && battery.percent <= LOW_BATTERY_PERCENT;
}
