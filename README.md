# php-ext-typst

This is a PHP extension for integrating the typst compiler with the PHP language.

Features:

- compile typst in-memory from PHP
- populate typst variables directly from PHP Values

## Building

You must have PHP headers installed in order to build. You can select a php version to build against by setting the `PHP` and `PHP_CONFIG` variables to specific instances of php and php-config.

```shell
$ cargo build --release
```

## Installation

Check `php-config` to locate the `--extension-dir` where the built extension should be installed.

## Testing

You can test the plugin with the PHP cli by adding the flag `-d extension=<path to extension>`.

## API

The `Typst` class is used to build the compiler environment, which includes the template body and all variables passed to the compiler.

```php
class Typst {
    // Creates a Typst class with an optional body.
    public function __construct(?string $body = null);

    // Set or overwrite the typst template body.
    public function body(string $body);

    // Set a json-encoded variable in the typst environment.
    public function json(string $key, string $value);

    // Set a csv-encoded variable in the typst environment.
    // If $use_headers is false, the typst variable will be an array of arrays.
    // If $use_headers is true, the typst variable will be an array of dict, using the headers as keys.
    public function csv(string $key, string $value, string $delimiter = ',', bool $use_headers = false);

    // Set a php variable in the typst environment.
    public function var(string $key, mixed $value);

    // Compile the current environment to PDF.
    // This will return a binary string on success or null on error.
    // This function can throw an Error on compilation failure.
    public function compile(): ?string;
}
```

## Example

```typst
Invoice #invoice_num

#client.name

#client.address.join("\n")

#table(
    columns: (1fr, 60pt, 60pt),
    ..services.map((line) => (
        line.title,
        str(line.rate),
        str(line.quantity),
    )).flatten()
)
```

```php
// Primitives
$invoice_num = 2091;

// Nested associative and numerically indexed arrays
$client = [
    'name' => 'ABC Corp',
    'address' => [
        '1000 Maple Ave',
        'Test Town, TN 12345',
    ],
];

// Object data
$services = [
    (object)[ 'title' => 'Example Service', 'rate' => 125, 'quantity' => 9.5 ],
    (object)[ 'title' => 'Example Service 2', 'rate' => 125, 'quantity' => 2 ],
];

$builder = new Typst(file_get_contents("./invoice.typ"));
$builder->var('invoice_num', $invoice_num);
$builder->var('client', $client);
$builder->var('services', $services);

try {
    $pdf_result = $builder->compile();

    header('Content-Disposition: inline; filename="invoice-' . $invoice_num . '.pdf"');
    header('Content-Type: application/pdf');
    echo $pdf_result;
}
catch (Exception $e) {
    echo $e;
}
```

