export function submitForm(handler) {
  return (event) => {
    event.preventDefault();
    handler(new FormData(event.currentTarget));
  };
}
