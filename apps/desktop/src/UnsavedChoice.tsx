// Unsaved changes before opening another show or making a new one: the same
// three choices as the close confirmation.
export default function UnsavedChoice(props: {
  /** "開く" or "新規を作る". */
  action: string;
  path?: string | null;
  error?: string | null;
  onSave: () => void;
  onDiscard: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="modal-back">
      <div className="modal">
        <h2>未保存の変更があります</h2>
        {props.path && <p>{props.path}</p>}
        <div className="actions">
          <button className="primary" onClick={props.onSave}>保存して{props.action}</button>
          <button className="danger" onClick={props.onDiscard}>保存せずに{props.action}</button>
          <button onClick={props.onCancel}>キャンセル</button>
        </div>
        {props.error && <p className="error">{props.error}</p>}
      </div>
    </div>
  );
}
