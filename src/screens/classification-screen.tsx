import { Badge } from "@astryxdesign/core/Badge";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { CheckboxInput } from "@astryxdesign/core/CheckboxInput";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Heading } from "@astryxdesign/core/Heading";
import { List, ListItem } from "@astryxdesign/core/List";
import { Text } from "@astryxdesign/core/Text";
import { VStack } from "@astryxdesign/core/VStack";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Mirrors `classify::Decision` (`src-tauri/src/classify.rs`), as serialized
 * by `device_scan::classify_suggest`. Serde's default representation for a
 * unit-only enum is a bare string of the variant name.
 */
type Decision = "Include" | "Skip" | "SkipStaleDuplicate";

/** Mirrors `device_scan::SuggestedFolder`. */
interface SuggestedFolder {
  decision: Decision;
  name: string;
}

function isIncludedByDefault(decision: Decision): boolean {
  return decision === "Include";
}

function decisionSummary(decision: Decision): string {
  switch (decision) {
    case "Include":
      return "Suggested: include";
    case "Skip":
      return "Suggested: skip";
    case "SkipStaleDuplicate":
      return "Suggested: skip — superseded by a newer copy under Android/media";
    default:
      return decision;
  }
}

interface ClassificationScreenProps {
  /**
   * The device serial to scan, e.g. lifted from `DeviceScreen`'s
   * `onDeviceSelected` seam (see `src/App.tsx`). Not wired to that seam yet
   * — this task only builds the screen itself; routing between screens is a
   * later concern.
   */
  serial: string;
}

export function ClassificationScreen({ serial }: ClassificationScreenProps) {
  const [suggestions, setSuggestions] = useState<SuggestedFolder[]>([]);
  const [included, setIncluded] = useState<Record<string, boolean>>({});
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [isSaved, setIsSaved] = useState(false);

  // Task 11's review noted its unmount-cancellation-guard only covered the
  // initial mount fetch, not retry-triggered fetches, since it recreated a
  // fresh `cancelled` flag per call but only the mount effect's cleanup ever
  // read it. Here every fetch (mount AND retry) bumps a shared generation
  // counter and checks against it before touching state, so a stale
  // in-flight request from an earlier click/mount can never clobber state
  // set by a newer one -- not just unmount.
  const fetchGenerationRef = useRef(0);

  const loadSuggestions = useCallback(() => {
    fetchGenerationRef.current += 1;
    const generation = fetchGenerationRef.current;
    setIsLoading(true);
    setError(null);
    setIsSaved(false);

    invoke<SuggestedFolder[]>("classify_suggest", { serial })
      .then((result) => {
        if (fetchGenerationRef.current !== generation) {
          return;
        }
        setSuggestions(result);
        setIncluded(
          Object.fromEntries(
            result.map((folder) => [
              folder.name,
              isIncludedByDefault(folder.decision),
            ])
          )
        );
      })
      .catch((err: unknown) => {
        if (fetchGenerationRef.current !== generation) {
          return;
        }
        setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (fetchGenerationRef.current !== generation) {
          return;
        }
        setIsLoading(false);
      });
  }, [serial]);

  useEffect(() => {
    loadSuggestions();
  }, [loadSuggestions]);

  const handleToggle = useCallback((name: string, checked: boolean) => {
    setIncluded((prev) => ({ ...prev, [name]: checked }));
  }, []);

  // No `folder_rules` persistence command exists yet (that's a later task —
  // see the design doc's `profile_save` concept). This is a local-state-only
  // placeholder seam: it just confirms the reviewed list in the UI instead
  // of writing anything to SQLite.
  const handleSave = useCallback(() => {
    setIsSaved(true);
  }, []);

  if (error) {
    return (
      <VStack gap={4}>
        <Heading level={1}>Review folders</Heading>
        <Banner
          description={error}
          endContent={<Button label="Retry" onClick={loadSuggestions} />}
          status="error"
          title="Failed to scan the device"
        />
      </VStack>
    );
  }

  if (isLoading) {
    return (
      <VStack gap={4}>
        <Heading level={1}>Review folders</Heading>
        <Text color="secondary">Scanning device storage…</Text>
      </VStack>
    );
  }

  if (suggestions.length === 0) {
    return (
      <EmptyState
        actions={<Button label="Retry" onClick={loadSuggestions} />}
        description="No top-level folders were found under the device's storage root."
        title="Nothing to classify"
      />
    );
  }

  const includedCount = Object.values(included).filter(Boolean).length;

  return (
    <VStack gap={4}>
      <Heading level={1}>Review folders</Heading>
      <Text color="secondary">
        {includedCount} of {suggestions.length} folders selected for backup.
        Suggestions are pre-checked based on the folder's contents — review and
        adjust before saving.
      </Text>
      {isSaved ? (
        <Banner
          description="This is a local review only for now — nothing is written to the device yet."
          status="success"
          title="Selections saved"
        />
      ) : null}
      <List hasDividers header={<Text type="label">Detected folders</Text>}>
        {suggestions.map((folder) => (
          <FolderListItem
            folder={folder}
            isIncluded={included[folder.name] ?? false}
            key={folder.name}
            onToggle={handleToggle}
          />
        ))}
      </List>
      <Button label="Save selections" onClick={handleSave} />
    </VStack>
  );
}

function FolderListItem({
  folder,
  isIncluded,
  onToggle,
}: {
  folder: SuggestedFolder;
  isIncluded: boolean;
  onToggle: (name: string, checked: boolean) => void;
}) {
  const checkboxRef = useRef<HTMLInputElement>(null);

  const handleChange = useCallback(
    (checked: boolean) => {
      onToggle(folder.name, checked);
    },
    [folder.name, onToggle]
  );

  return (
    <ListItem
      description={decisionSummary(folder.decision)}
      endContent={
        folder.decision === "SkipStaleDuplicate" ? (
          <Badge label="Stale duplicate" variant="warning" />
        ) : undefined
      }
      interactiveRef={checkboxRef}
      label={folder.name}
      startContent={
        <CheckboxInput
          isLabelHidden
          label={`Include ${folder.name}`}
          onChange={handleChange}
          ref={checkboxRef}
          value={isIncluded}
        />
      }
    />
  );
}
