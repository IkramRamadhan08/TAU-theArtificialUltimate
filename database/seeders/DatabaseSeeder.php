<?php

namespace Database\Seeders;

use App\Models\User;
use Illuminate\Database\Console\Seeds\WithoutModelEvents;
use Illuminate\Database\Seeder;
use Illuminate\Support\Facades\Hash;

class DatabaseSeeder extends Seeder
{
    use WithoutModelEvents;

    /**
     * Seed the application's database.
     */
    public function run(): void
    {
        $email = env('LAUNDRYKU_ADMIN_EMAIL', 'test@example.com');
        $password = env('LAUNDRYKU_ADMIN_PASSWORD', 'password');

        User::updateOrCreate(
            ['email' => $email],
            [
                'name' => 'Test User',
                'password' => Hash::make($password),
                'email_verified_at' => now(),
            ],
        );
    }
}
