CREATE TABLE `devices` (
	`display_name` text NOT NULL,
	`first_seen` integer NOT NULL,
	`last_seen` integer NOT NULL,
	`serial` text PRIMARY KEY NOT NULL
);
--> statement-breakpoint
CREATE TABLE `folder_rules` (
	`decision` text NOT NULL,
	`device_serial` text NOT NULL,
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`path` text NOT NULL,
	`source` text NOT NULL,
	`updated_at` integer NOT NULL,
	FOREIGN KEY (`device_serial`) REFERENCES `devices`(`serial`) ON UPDATE no action ON DELETE no action
);
--> statement-breakpoint
CREATE INDEX `folder_rules_device_serial_idx` ON `folder_rules` (`device_serial`);--> statement-breakpoint
CREATE INDEX `folder_rules_path_idx` ON `folder_rules` (`path`);--> statement-breakpoint
CREATE UNIQUE INDEX `folder_rules_device_serial_path_unique` ON `folder_rules` (`device_serial`,`path`);--> statement-breakpoint
CREATE TABLE `run_items` (
	`bytes_transferred` integer,
	`error_message` text,
	`file_count` integer,
	`finished_at` integer,
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`path` text NOT NULL,
	`run_id` integer NOT NULL,
	`status` text NOT NULL,
	FOREIGN KEY (`run_id`) REFERENCES `runs`(`id`) ON UPDATE no action ON DELETE no action
);
--> statement-breakpoint
CREATE INDEX `run_items_run_id_idx` ON `run_items` (`run_id`);--> statement-breakpoint
CREATE INDEX `run_items_path_idx` ON `run_items` (`path`);--> statement-breakpoint
CREATE TABLE `runs` (
	`device_serial` text NOT NULL,
	`finished_at` integer,
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`started_at` integer NOT NULL,
	`status` text NOT NULL,
	`type` text NOT NULL,
	FOREIGN KEY (`device_serial`) REFERENCES `devices`(`serial`) ON UPDATE no action ON DELETE no action
);
--> statement-breakpoint
CREATE INDEX `runs_device_serial_idx` ON `runs` (`device_serial`);