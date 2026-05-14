# Scope

## Goal

Generate a CSV file containing 10 BACnet data points that can be imported into a BACnet device or used as test fixture data.

## Deliverables

- A single file `bacnet_points.csv` with a header row and exactly 10 data rows.
- Each row represents one BACnet object with the following columns:
  - `object_type` — e.g. `analog-input`, `analog-output`, `binary-input`, `binary-output`, `analog-value`, `binary-value`
  - `object_instance` — integer instance number (unique within the file)
  - `object_name` — human-readable name, e.g. `Zone_Temp_1`
  - `description` — short description of what the point represents
  - `units` — engineering units where applicable (e.g. `degrees-celsius`, `percent`, `no-units`)
  - `present_value` — a realistic sample value

## Constraints

- All 10 points must have distinct `object_instance` values.
- Include a mix of at least 3 different `object_type` values.
- Values must be realistic for a building-automation context (HVAC, lighting, etc.).
- No external dependencies — the output is a plain text CSV file.

## Out of scope

- Writing to a live BACnet device or network.
- BACnet/IP or MSTP communication.
- More than 10 points.
