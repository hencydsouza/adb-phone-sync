CREATE TABLE `devices` (
	`serial` text PRIMARY KEY NOT NULL,
	`display_name` text NOT NULL,
	`first_seen` integer NOT NULL,
	`last_seen` integer NOT NULL
);
--> statement-breakpoint
CREATE TABLE `folder_rules` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`device_serial` text NOT NULL,
	`path` text NOT NULL,
	`decision` text NOT NULL,
	`source` text NOT NULL,
	`updated_at` integer NOT NULL,
	FOREIGN KEY (`device_serial`) REFERENCES `devices`(`serial`) ON UPDATE no action ON DELETE no action
);
--> statement-breakpoint
CREATE TABLE `run_items` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`run_id` integer NOT NULL,
	`path` text NOT NULL,
	`status` text NOT NULL,
	`bytes_transferred` integer,
	`file_count` integer,
	`error_message` text,
	`finished_at` integer,
	FOREIGN KEY (`run_id`) REFERENCES `runs`(`id`) ON UPDATE no action ON DELETE no action
);
--> statement-breakpoint
CREATE TABLE `runs` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`device_serial` text NOT NULL,
	`type` text NOT NULL,
	`started_at` integer NOT NULL,
	`finished_at` integer,
	`status` text NOT NULL,
	FOREIGN KEY (`device_serial`) REFERENCES `devices`(`serial`) ON UPDATE no action ON DELETE no action
);
