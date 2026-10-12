-- An input a Task requested from its caller, and the caller's answer.
CREATE TABLE tasks.task_input (
    task_id uuid NOT NULL REFERENCES tasks.task (id) ON DELETE CASCADE,
    request_key text NOT NULL,
    method text NOT NULL,
    params jsonb NOT NULL,
    response jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    responded_at timestamptz,
    PRIMARY KEY (task_id, request_key),
    CONSTRAINT task_input_key CHECK (tasks.valid_key(request_key)),
    CONSTRAINT task_input_method CHECK (tasks.valid_key(method)),
    CONSTRAINT task_input_params CHECK (jsonb_typeof(params) = 'object'),
    CONSTRAINT task_input_response CHECK (
        response IS NULL OR jsonb_typeof(response) = 'object'
    ),
    CONSTRAINT task_input_answered CHECK ((response IS NULL) = (responded_at IS NULL))
);

ALTER TABLE tasks.task_input ENABLE ROW LEVEL SECURITY;

-- The parent read applies the Task policy, so a server sees only its own inputs.
CREATE POLICY task_input_server ON tasks.task_input TO veoveo_server
USING (EXISTS (SELECT FROM tasks.task WHERE task.id = task_input.task_id))
WITH CHECK (EXISTS (SELECT FROM tasks.task WHERE task.id = task_input.task_id));

GRANT SELECT, INSERT, UPDATE, DELETE ON tasks.task_input TO veoveo_server;
